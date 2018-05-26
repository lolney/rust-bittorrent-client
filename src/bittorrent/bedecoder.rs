use bittorrent::ParseError;
use std::collections::HashMap;

use bittorrent::utils::parse_i64;
use bittorrent::BencodeT;

/// # Arguments
/// # * `inbytes` - bencoded data
pub fn parse(inbytes: &[u8]) -> Result<BencodeT, ParseError> {
    let (r, _) = _parse(inbytes, 0)?;
    return Ok(r);
}

/// # Arguments
/// # * `inbytes` - bencoded data
fn _parse(inbytes: &[u8], i: usize) -> Result<(BencodeT, usize), ParseError> {
    match inbytes[i] as char {
        'i' => parse_int(inbytes, i + 1),
        'l' => parse_list(inbytes, i + 1),
        'd' => parse_dic(inbytes, i + 1),
        '0'...'9' => parse_str(inbytes, i),
        _ => Err(parse_error!("Incorrect format at {:?} in {:?}", i, inbytes)),
    }
}

/// # Arguments
/// # * `inbytes` - bencoded data
/// # * `i` - index where the bencoded i64 integer begins, discluding leading 'i'
fn parse_int(inbytes: &[u8], i: usize) -> Result<(BencodeT, usize), ParseError> {
    let mut j = i;
    loop {
        if j >= inbytes.len() {
            return Err(parse_error!("Incorrect format; exceeded buffer length"));
        }
        if inbytes[j] as char == 'e' {
            if j == i {
                return Err(parse_error!("Empty integer"));
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
            return Err(parse_error!(
                "Expected int; found character {:?}",
                inbytes[j] as char
            ));
        }
        j = j + 1;
    }
    // Overflows with > max i32
    let int = ascii_utf8_to_i64(&inbytes[i..j]);
    Ok((BencodeT::Integer(int), j + 1))
}

fn parse_list(inbytes: &[u8], i: usize) -> Result<(BencodeT, usize), ParseError> {
    let mut j = i;
    let mut vec: Vec<BencodeT> = Vec::new();
    loop {
        if j >= inbytes.len() {
            return Err(parse_error!("Incorrect format; exceeded buffer length"));
        }
        if inbytes[j] as char == 'e' {
            break;
        }
        let (bt, newj) = _parse(inbytes, j)?;
        j = newj;
        vec.push(bt);
    }

    let l = BencodeT::List(vec);
    Ok((l, j + 1))
}

fn parse_dic(inbytes: &[u8], i: usize) -> Result<(BencodeT, usize), ParseError> {
    let mut j = i;
    let mut hm = HashMap::new();
    loop {
        if j >= inbytes.len() {
            return Err(parse_error!("Incorrect format; exceeded buffer length"));
        }
        if inbytes[j] as char == 'e' {
            break;
        }

        let (b1, j2) = _parse(inbytes, j)?;
        let mut key = Default::default();
        match b1 {
            BencodeT::String(v) => {
                key = v;
            }
            _ => return Err(parse_error!("Non-string type as key")),
        };

        let (value, j3) = _parse(inbytes, j2)?;
        j = j3;

        hm.insert(key, value);
    }

    Ok((BencodeT::Dictionary(hm), j + 1))
}

fn is_digit(d: u8) -> bool {
    match d as char {
        '0'...'9' => true,
        _ => false,
    }
}

/// # Arguments
/// # * `buffer` - ascii-encoded unsigned integer
/// # Returns
/// # bytes intrepreted as an unsigned integer
fn ascii_utf8_to_usize(buffer: &[u8]) -> usize {
    let as_string = String::from_utf8(buffer.to_vec()).unwrap();
    return as_string.parse::<usize>().unwrap();
}

fn ascii_utf8_to_i64(buffer: &[u8]) -> i64 {
    let as_string = String::from_utf8(buffer.to_vec()).unwrap();
    return parse_i64(as_string).unwrap();
}

fn utf8_to_string(bytes: &[u8]) -> BencodeT {
    let vector: Vec<u8> = Vec::from(bytes);
    match String::from_utf8(vector) {
        Ok(s) => BencodeT::String(s),
        Err(e) => BencodeT::ByteString(Vec::from(bytes)),
    }
}

fn parse_str(inbytes: &[u8], i: usize) -> Result<(BencodeT, usize), ParseError> {
    let mut j = i;
    while is_digit(inbytes[j]) {
        j = j + 1;
    }
    if inbytes[j] as char != ':' {
        return Err(parse_error!(
            "Incorrect format while parsing string: at {:?} in {:?}",
            j,
            inbytes
        ));
    }
    // TODO: error handling for size larger than usize
    let l: usize = ascii_utf8_to_usize(&inbytes[i..j]);
    j = j + 1;
    let bencoded = utf8_to_string(&inbytes[j..j + l]);
    Ok((bencoded, j + l))
}

#[cfg(test)]
mod tests {

    use bittorrent::bedecoder::*;
    use std::collections::hash_map::DefaultHasher;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};

    use bittorrent::bencoder::encode;
    use bittorrent::utils::create_strings;
    use bittorrent::BencodeT;
    use std::error::Error;

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
    fn bstrings() {
        let mut vec = Vec::new();
        for i in (0..255) {
            vec.push(i as u8);
        }
        match parse(&vec).unwrap() {
            BencodeT::ByteString(ref vec) => (),
            _ => assert!(false),
        }
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
            parse("i333333".as_bytes()).unwrap_err().description(),
            parse_error!("Incorrect format; exceeded buffer length").description()
        );
        assert_eq!(
            parse("i3-3e".as_bytes()).unwrap_err().description(),
            parse_error!("Expected int; found character {:?}", '-' as char).description()
        );
        assert_eq!(
            parse("i7373777788888989898989899898ae".as_bytes())
                .unwrap_err()
                .description(),
            parse_error!("Expected int; found character {:?}", 'a').description()
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
            parse("li3e".as_bytes()).unwrap_err().description(),
            parse_error!("Incorrect format; exceeded buffer length").description()
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
            dictstring.push_str(&String::from_utf8(hash).unwrap());
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
