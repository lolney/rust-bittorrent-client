use bittorrent::BencodeT;
use std::collections::{BTreeMap, HashMap};

/// Entry point for serializing (bencoding) BencodeT objects
pub fn encode(bencobj: &BencodeT) -> Vec<u8> {
    match bencobj {
        &BencodeT::String(ref string) => encode_string(string.as_str()),
        &BencodeT::Integer(int) => encode_int(int),
        &BencodeT::Dictionary(ref hm) => encode_dic(&hm),
        &BencodeT::List(ref vec) => encode_list(&vec),
        &BencodeT::ByteString(ref vec) => encode_bstring(&vec),
    }
}

fn encode_string(instring: &str) -> Vec<u8> {
    encode_bstring(instring.as_bytes())
}

fn encode_int(int: i64) -> Vec<u8> {
    let mut vec = Vec::new();
    vec.push('i' as u8);
    vec.extend_from_slice(int.to_string().as_bytes());
    vec.push('e' as u8);
    return vec;
}

fn encode_dic(hm: &HashMap<String, BencodeT>) -> Vec<u8> {
    let mut vec = Vec::new();
    vec.push('d' as u8);
    let ordered: BTreeMap<_, _> = hm.iter().collect();
    for (key, value) in ordered.iter() {
        vec.extend_from_slice(&encode_string(key));
        vec.extend_from_slice(&encode(value));
    }
    vec.push('e' as u8);
    return vec;
}

fn encode_list(list: &Vec<BencodeT>) -> Vec<u8> {
    let mut vec = Vec::new();
    vec.push('l' as u8);
    for elem in list {
        vec.extend_from_slice(&encode(elem));
    }
    vec.push('e' as u8);
    return vec;
}

fn encode_bstring(slice: &[u8]) -> Vec<u8> {
    let mut vec = Vec::from(slice.len().to_string().as_bytes());
    vec.push(':' as u8);
    vec.extend_from_slice(slice);
    return vec;
}

#[cfg(test)]
mod tests {

    use bittorrent::bencoder::*;
    use bittorrent::utils::{create_ints, create_strings};
    use bittorrent::BencodeT;
    use std::collections::HashMap;

    #[test]
    fn strings() {
        let (strings, bstrings) = create_strings();
        for (bstring, string) in bstrings.iter().zip(strings) {
            assert_eq!(string.as_bytes(), encode(bstring).as_slice());
        }
    }

    #[test]
    fn ints() {
        let (strings, ints) = create_ints();
        for (int, string) in ints.iter().zip(strings) {
            assert_eq!(string.as_bytes(), encode(int).as_slice());
        }
    }

    #[test]
    fn lists() {
        let (sstrings, sbencoded) = create_strings();
        let (istrings, ibencoded) = create_ints();
        let list = BencodeT::List(sbencoded.clone());
        assert_eq!(encode_list(&sbencoded), encode(&list).as_slice());

        let string = "li1ei2ei3ee";
        let list = BencodeT::List(
            vec![1, 2, 3]
                .into_iter()
                .map(|i| BencodeT::Integer(i))
                .collect(),
        );
        assert_eq!(string.as_bytes(), encode(&list).as_slice());
    }

    #[test]
    fn dics() {
        let (sstrings, sbencoded) = create_strings();
        let (istrings, ibencoded) = create_ints();
        let mut hm = HashMap::new();
        for (i, sb) in sbencoded.iter().enumerate() {
            hm.insert(i.to_string(), sb.clone());
        }
        let bhm = BencodeT::Dictionary(hm.clone());
        assert_eq!(encode_dic(&hm), encode(&bhm).as_slice());

        let string = "d1:gd1:ei8e1:fi9eee";
        let mut dic1 = HashMap::new();
        dic1.insert(String::from("e"), BencodeT::Integer(8));
        dic1.insert(String::from("f"), BencodeT::Integer(9));
        let mut dic2 = HashMap::new();
        dic2.insert(String::from("g"), BencodeT::Dictionary(dic1));
        assert_eq!(
            string.as_bytes(),
            encode(&BencodeT::Dictionary(dic2)).as_slice()
        );
    }
}
