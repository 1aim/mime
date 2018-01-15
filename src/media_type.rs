use std::collections::HashMap;
use std::iter::{Iterator, ExactSizeIterator};
use std::slice;
use std::marker::PhantomData;
use std::ops::Deref;
use std::fmt::{self, Debug};



use error::ParserErrorRef;
use name::{Name, CHARSET};
use value::{Value, UTF_8, UTF8};


use parse::{Spec, ParseResult, ParamIndices, parse, validate};


#[derive(Clone, Debug)]
pub struct MediaType<S: Spec> {
    inner: AnyMediaType,
    _spec: PhantomData<S>
}

impl<S> MediaType<S>
    where S: Spec
{
    pub fn parse(input: &str) -> Result<Self, ParserErrorRef> {
        let parse_result: ParseResult = parse::<S>(input)?;
        let media_type: AnyMediaType = parse_result.into();
        Ok(MediaType { inner: media_type, _spec: PhantomData })
    }

    pub fn validate(input: &str) -> bool {
        validate::<S>(input)
    }
}

impl<S1, S2> PartialEq<MediaType<S2>> for MediaType<S1>
    where S1: Spec, S2: Spec
{
    // Spec is just about parsing/normalizing etc. we can compare independent of it
    fn eq(&self, other: &MediaType<S2>) -> bool {
        self.deref() == other.deref()
    }
}

impl<S> Deref for MediaType<S>
    where S: Spec
{
    type Target = AnyMediaType;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<S> fmt::Display for MediaType<S>
    where S: Spec
{

    fn fmt(&self, fter: &mut fmt::Formatter) -> fmt::Result {
        write!(fter, "{}", self.as_str_repr())
    }
}


impl<S> Into<AnyMediaType> for MediaType<S>
    where S: Spec
{
    fn into(self) -> AnyMediaType {
        self.inner
    }
}

#[derive(Clone,  Debug)]
pub struct AnyMediaType {
    //idx layout
    //                              /plus_idx if there is no suffix, buffer.len() if there are no parameters
    //                             /
    //  type /  subtype  + suffix  ; <space>  param_name    =   param_value  ; <space> pn = pv
    //       \           \         \          \             \                \          \
    //        \slash_idx  \plus_idx \          \             \eon_idx         \ofv_idx   \prev eov_idx + 2
    //                               \eot_idx   \prev eov_idx +2 == eot_idx + 2 if first param
    buffer: String,
    slash_idx: usize,
    /// is equal the end_type_idx if there is no plus
    //plus_idx: usize,
    /// it is the index behind the last character of the subtype(inkl. suffix) which is equal to the
    /// index of the ";" of the first parameter or the len of the buffer if there are no parameter
    end_of_type: usize,
    params: Vec<ParamIndices>
}

impl AnyMediaType {

    pub fn type_(&self) -> Name {
        Name::new_unchecked(&self.buffer[..self.slash_idx])
    }

    pub fn subtype(&self) -> Name {
        Name::new_unchecked(&self.buffer[self.slash_idx+1..self.end_of_type])
        //Name::new_unchecked(&self.buffer[self.slash_idx+1..self.plus_idx])
    }

//    pub fn suffix(&self) -> Option<Name> {
//        let suffix_start = self.plus_idx+1;
//        let end_idx = self.end_of_type;
//        if suffix_start < end_idx {
//            Some(Name::new_unchecked(&self.buffer[suffix_start..end_idx]))
//        } else {
//            None
//        }
//    }

    pub fn get_param<'a, N>(&'a self, attr: N) -> Option<Value<'a>>
        where N: PartialEq<Name<'a>>
    {
        self.params()
            .find(|nv| attr == nv.0)
            .map(|(_name, value)| value)
    }

    pub fn params(&self) -> Params {
        Params {
            iter: self.params.iter(),
            source: self.buffer.as_str()
        }
    }

    pub fn as_str_repr(&self) -> &str {
        self.buffer.as_str()
    }

    pub fn has_utf8_charset(&self) -> bool {
        self.get_param(CHARSET)
            .map(|cs_param| {
                //FIXME use eq_ascii_case_insensitive
                cs_param == UTF_8 || cs_param == UTF8
            })
            .unwrap_or(false)
    }

}

impl fmt::Display for AnyMediaType {

    fn fmt(&self, fter: &mut fmt::Formatter) -> fmt::Result {
        write!(fter, "{}", self.as_str_repr())
    }
}

impl PartialEq for AnyMediaType {
    fn eq(&self, other: &AnyMediaType) -> bool {
        if self.type_() != other.type_()
            || self.subtype() != other.subtype()
            //|| self.suffix() != other.suffix()
        {
            return false;
        } else {
            let len = self.params.len();
            let other_len = other.params.len();
            if len != other_len { return false; }
            match len {
                0 => true,

                //OPTIMIZATION: most media types have very little parameter, so we can avoid
                // the "costy order independent comparsion" for them
                1 => {
                    let (name, value) = self.params().next().unwrap();
                    let (other_name, other_value) = other.params().next().unwrap();
                    return name == other_name && value == other_value
                },
                //FIXME check to which number it makes sense 2?/3?
                2 => {
                    let mut params = self.params();
                    let mut other_params = other.params();
                    let (name1, value1) = params.next().unwrap();
                    let (other_name1, other_value1) = other_params.next().unwrap();
                    let (name2, value2) = params.next().unwrap();
                    let (other_name2, other_value2) = other_params.next().unwrap();
                    if name1 == other_name1 {
                        return value1 == other_value1
                            && name2 == other_name2 && value2 == other_value2
                    } else {
                        return
                            name1 == other_name2 && value1 == other_value2
                                && name2 == other_name1 && value2 == other_value1
                    }
                },
                _ => {
                    //TODO Optimized use on stack map, sort compare?
                    let map = self.params().collect::<HashMap<_, _>>();
                    // we already checked that the len of both is the same
                    // so if all params of other are in map they are equal
                    other.params()
                        .all(|(other_name, other_value)| {
                            map.get(&other_name)
                                .map(|value| other_value == *value)
                                .unwrap_or(false)
                        })
                }
            }
        }
    }
}


impl<'a> From<ParseResult<'a>> for AnyMediaType {

    fn from(pres: ParseResult) -> Self {
        let mut buffer;
        if pres.params.len() == 0 {
            buffer = pres.input[..pres.repr_len()].to_ascii_lowercase();
        } else {
            buffer = String::from(&pres.input[..pres.repr_len()]);

            buffer[0..pres.end_of_type_idx]
                .make_ascii_lowercase();

            for param_indices in pres.params.iter() {
                buffer[param_indices.start..param_indices.eq_idx].make_ascii_lowercase();
            }
        }

        AnyMediaType {
            buffer,
            slash_idx: pres.slash_idx,
            end_of_type: pres.end_of_type_idx,
            params: pres.params
        }
    }
}





#[derive(Clone)]
pub struct Params<'a> {
    source: &'a str,
    iter: slice::Iter<'a, ParamIndices>
}

impl<'a> Iterator for Params<'a> {
    type Item = (Name<'a>, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
            .map(|pidx| {
                //TODO OPTIMIZE:
                //   using unsafe slace removes ca. 30% of the comparsion time
                //   (for text/plain; param=value)

                let name = &self.source[pidx.start..pidx.eq_idx];
                let value = &self.source[pidx.eq_idx+1..pidx.end];
                (Name::new_unchecked(name), Value::new_unchecked(value))
            })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a> ExactSizeIterator for Params<'a> {
    #[inline]
    fn len(&self) -> usize {
        self.iter.len()
    }
}

impl<'a> Debug for Params<'a> {

    fn fmt(&self, fter: &mut fmt::Formatter) -> fmt::Result {
        let metoo = self.clone();
        fter.debug_list()
            .entries(metoo)
            .finish()
    }
}



#[cfg(test)]
mod test {
    use super::{AnyMediaType, MediaType};
    use ::parse::{AnySpec, StrictSpec};

    #[test]
    fn simple_parse() {
        let mt: MediaType<_> = assert_ok!(MediaType::<AnySpec>::parse("text/plain; charset=utf-8"));
        assert!(mt.has_utf8_charset());
        assert_eq!(mt.as_str_repr(), "text/plain; charset=utf-8");
    }

    #[test]
    fn parsing_does_not_normalizes_whitespaces() {
        let mt: MediaType<_> = assert_ok!(MediaType::<AnySpec>::parse("text/plain   ;charset=utf-8"));
        assert!(mt.has_utf8_charset());
        assert_eq!(mt.as_str_repr(), "text/plain   ;charset=utf-8");
    }

    #[test]
    fn parsing_does_not_normalized_utf8() {
        let mt: MediaType<_> = assert_ok!(MediaType::<AnySpec>::parse("text/plain; charset=utf8"));
        assert!(mt.has_utf8_charset());
        assert_eq!(mt.as_str_repr(), "text/plain; charset=utf8");
    }


    #[test]
    fn params_iter_behaviour() {
        let mt: MediaType<AnySpec> = assert_ok!(MediaType::parse("test/plain; c1=abc; c2=def"));
        let mut iter = mt.params();
        assert_eq!(iter.len(), 2);
        assert_eq!(iter.size_hint(), (2, Some(2)));

        let p1 = iter.next().unwrap();
        assert_eq!(p1.0, "c1");
        assert_eq!(p1.1, "abc");
        assert_eq!(iter.len(), 1);
        assert_eq!(iter.size_hint(), (1, Some(1)));

        let p1 = iter.next().unwrap();
        assert_eq!(p1.0, "c2");
        assert_eq!(p1.1, "def");
        assert_eq!(iter.len(), 0);
        assert_eq!(iter.size_hint(), (0, Some(0)));

        assert_eq!(iter.next(), None);
    }

    #[test]
    fn any_media_type_eq() {
        let mt1: AnyMediaType = assert_ok!(
            MediaType::<AnySpec>::parse("text/plain; p1=\"a\"; p2=b")).into();
        let mt2: AnyMediaType = assert_ok!(
            MediaType::<AnySpec>::parse("text/plain; p2=\"b\"; p1=a")).into();

        assert_eq!(mt1, mt2);
    }

    #[test]
    fn media_type_eq_different_spec() {
        let mt1 = assert_ok!(
            MediaType::<AnySpec>::parse("text/plain; p1=\"a\"; p2=b"));
        let mt2 = assert_ok!(
            MediaType::<StrictSpec>::parse("text/plain; p2=\"b\"; p1=a"));

        assert_eq!(mt1, mt2);
    }
}