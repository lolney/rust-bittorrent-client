use std::collections::HashMap;

use bittorrent::BencodeT;
use bittorrent::utils::{create_strings, parse_i64};
/*
#[derive(Debug)]
struct ParseError;

impl Error for ParseError {
    fn description(&self) -> &str {
        "Something bad happened"
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Oh no, something bad went down")
    }
}*/

/// # Arguments
/// # * `inbytes` - bencoded data
pub fn parse(inbytes: &[u8]) -> Result<BencodeT, String> {
    let (r, _) = _parse(inbytes, 0);
    return r;
}

/// # Arguments
/// # * `inbytes` - bencoded data
fn _parse(inbytes: &[u8], i: usize) -> (Result<BencodeT, String>, usize) {
    match inbytes[i] as char {
        'i' => parseInt(inbytes, i + 1),
        'l' => parseList(inbytes, i + 1),
        'd' => parseDic(inbytes, i + 1),
        '0'...'9' => parseStr(inbytes, i),
        _ => (
            Err(format!("Incorrect format at {:?} in {:?}", i, inbytes)),
            0,
        ),
    }
}

/// # Arguments
/// # * `inbytes` - bencoded data
/// # * `i` - index where the bencoded i64 integer begins, discluding leading 'i'
fn parseInt(inbytes: &[u8], i: usize) -> (Result<BencodeT, String>, usize) {
    let mut j = i;
    while true {
        if j >= inbytes.len() {
            return (
                Err(String::from("Incorrect format; exceeded buffer length")),
                j,
            );
        }
        if inbytes[j] as char == 'e' {
            if j == i {
                return (Err(String::from("Empty integer")), j);
            }
            break;
        }
        let allowed = match inbytes[j] as char {
            '-' => if j == i {
                true
            } else {
                false
            },
            '0'...'9' => true,
            _ => false,
        };
        if !allowed {
            return (
                Err(format!(
                    "Expected int; found character {:?}",
                    inbytes[j] as char
                )),
                j,
            );
        }
        j = j + 1;
    }
    // Overflows with > max i32
    let int = ascii_utf8_to_i64(&inbytes[i..j]);
    (Ok(BencodeT::Integer(int)), j + 1)
}

fn parseList(inbytes: &[u8], i: usize) -> (Result<BencodeT, String>, usize) {
    let mut j = i;
    let mut vec: Vec<BencodeT> = Vec::new();
    while true {
        if j >= inbytes.len() {
            return (
                Err(String::from("Incorrect format; exceeded buffer length")),
                j,
            );
        }
        if inbytes[j] as char == 'e' {
            break;
        }
        let (res, newj) = _parse(inbytes, j);
        j = newj;
        match res {
            Ok(bt) => {
                vec.push(bt);
            }
            Err(error) => {
                return (Err(error), j);
            }
        }
    }

    let l = BencodeT::List(vec);
    (Ok(l), j + 1)
}

fn parseDic(inbytes: &[u8], i: usize) -> (Result<BencodeT, String>, usize) {
    let mut j = i;
    let mut hm = HashMap::new();
    while true {
        if j >= inbytes.len() {
            return (
                Err(String::from("Incorrect format; exceeded buffer length")),
                j,
            );
        }
        if inbytes[j] as char == 'e' {
            break;
        }

        let (b1, j2) = _parse(inbytes, j);
        let mut key = String::from("");
        match b1 {
            Ok(b) => match b {
                BencodeT::String(v) => {
                    key = v;
                }
                _ => return (Err(String::from("Non-string type as key")), j),
            },
            Err(error) => {
                return (Err(error), j);
            }
        };

        let (b2, j3) = _parse(inbytes, j2);
        j = j3;
        let value = match b2 {
            Ok(b) => b,
            Err(error) => {
                return (Err(error), j);
            }
        };

        hm.insert(key, value);
    }

    (Ok(BencodeT::Dictionary(hm)), j + 1)
}

fn isDigit(d: u8) -> bool {
    match d as char {
        '0'...'9' => true,
        _ => false,
    }
}

/// # Arguments
/// # * `buffer` - ascii bytes
/// # Returns
/// # bytes intrepreted as an unsigned integer
fn ascii_utf8_to_usize(buffer: &[u8]) -> usize {
    let as_string = utf8_to_string(buffer);
    return as_string.parse::<usize>().unwrap();
}

// No std library support for generic integers
fn ascii_utf8_to_i64(buffer: &[u8]) -> i64 {
    let as_string = utf8_to_string(buffer);
    return parse_i64(as_string).unwrap();
}

// The 'pieces' string in the info dictionary
// probably contains invalid utf-8 -
// It's a bunch of hashes
fn utf8_to_string(bytes: &[u8]) -> String {
    let vector: Vec<u8> = Vec::from(bytes);
    return unsafe { String::from_utf8_unchecked(vector) };
}

fn parseStr(inbytes: &[u8], i: usize) -> (Result<BencodeT, String>, usize) {
    let mut j = i;
    while isDigit(inbytes[j]) {
        j = j + 1;
    }
    if inbytes[j] as char != ':' {
        return (
            Err(format!(
                "Incorrect format whilte parsing string: at {:?} in {:?}",
                j, inbytes
            )),
            j,
        );
    }
    // TODO: error handling for size larger than usize
    let l: usize = ascii_utf8_to_usize(&inbytes[i..j]);
    j = j + 1;
    let string = utf8_to_string(&inbytes[j..j + l]);
    (Ok(BencodeT::String(string)), j + l)
}

#[cfg(test)]
mod tests {

    use bittorrent::bedecoder::*;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    use bittorrent::BencodeT;
    use bittorrent::bencoder::encode;
    use bittorrent::utils::{create_strings};

    #[test]
    fn strings() {
        assert_eq!(
            parse("5:abcde".as_bytes()).unwrap(),
            BencodeT::String("abcde".to_string())
        );
        assert_eq!(
            parse("26:abcdefghijklmnopQRSTUVWxyz".as_bytes()).unwrap(),
            BencodeT::String("abcdefghijklmnopQRSTUVWxyz".to_string())
        );
    }

    #[test]
    // TODO: larger than i64
    fn digits_basic() {
        assert_eq!(parse("i3e".as_bytes()).unwrap(), BencodeT::Integer(3));
        assert_eq!(parse("i-3e".as_bytes()).unwrap(), BencodeT::Integer(-3));
    }

    #[test]
    fn digits_errors() {
        assert_eq!(
            parse("i333333".as_bytes()),
            Err(String::from("Incorrect format; exceeded buffer length"))
        );
        assert_eq!(
            parse("i3-3e".as_bytes()),
            Err(format!("Expected int; found character {:?}", '-' as char))
        );
        assert_eq!(
            parse("i7373777788888989898989899898ae".as_bytes()),
            Err(format!("Expected int; found character {:?}", 'a'))
        );
    }

    #[test]
    fn digits_maxi64() {
        assert_eq!(
            parse("i9223372036854775807e".as_bytes()).unwrap(),
            BencodeT::Integer(9223372036854775807)
        );
    }

    #[test]
    fn digits_maxi32() {
        assert_eq!(
            parse("i2147483649e".as_bytes()).unwrap(),
            BencodeT::Integer(2147483649)
        );
        assert_eq!(
            parse("i2147483647e".as_bytes()).unwrap(),
            BencodeT::Integer(2147483647)
        );
    }

    #[test]
    fn list_errors() {
        assert_eq!(
            parse("li3e".as_bytes()),
            Err(String::from("Incorrect format; exceeded buffer length"))
        );
    }

    #[test]
    fn lists() {
        let l1 = BencodeT::List(Vec::new());
        assert_eq!(parse("le".as_bytes()).unwrap(), l1);

        let bi = BencodeT::Integer(1234);
        let l2 = BencodeT::List(vec![bi]);
        assert_eq!(parse("li1234ee".as_bytes()).unwrap(), l2);

        let mut v = Vec::new();
        let mut s = String::from("l");
        for i in 1..100 {
            let bi = BencodeT::Integer(i);
            s.push('i');
            s.push_str(i.to_string().as_str());
            s.push('e');
            v.push(bi);
        }
        s.push('e');
        let l3 = BencodeT::List(v);
        assert_eq!(parse(s.as_bytes()).unwrap(), l3);

        let l4 = BencodeT::List(vec![l1, l2]);
    }

    fn calculate_hash<T: Hash>(t: &T) -> String {
        let mut s = DefaultHasher::new();
        t.hash(&mut s);
        s.finish().to_string()
    }

    fn create_dictionary(keys: Vec<&'static str>, values: Vec<BencodeT>) -> (String, BencodeT) {
        let mut hm = HashMap::new();
        let mut dictstring = String::from("d");
        for (key, value) in keys.iter().zip(values.iter()) {
            let hash = encode(&BencodeT::String(calculate_hash(&key)));
            dictstring.push_str(hash.as_str());
            dictstring.push_str(key); // keys are the formatted version of the Bencoded objects
            hm.insert(calculate_hash(&key), value.clone());
        }
        dictstring.push('e');

        let bdict = BencodeT::Dictionary(hm);
        (dictstring, bdict)
    }

    #[test]
    fn dictionaries() {
        let (strings, bstrings) = create_strings();
        let (dictstring, bdict) = create_dictionary(strings, bstrings);
        assert_eq!(parse(dictstring.as_bytes()).unwrap(), bdict);

        /*
            let mut keys = Vec::new();
            let mut values = Vec::new();
            for i in 1..10 {
                keys.push(dictstring.as_str());
                values.push(bdict);
            }
            let (dictstring, bdict) = create_dictionary(keys, values);
            assert_eq!(parse(dictstring.as_bytes()).unwrap(), bdict);*/
    }
}
