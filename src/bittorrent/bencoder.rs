use std::collections::{BTreeMap, HashMap};
use bittorrent::BencodeT;

pub fn encode(bencobj: &BencodeT) -> String {
    match bencobj {
        &BencodeT::String(ref string) => encode_string(string.as_str()),
        &BencodeT::Integer(int) => encode_int(int),
        &BencodeT::Dictionary(ref hm) => encode_dic(&hm),
        &BencodeT::List(ref vec) => encode_list(&vec),
    }
}

fn encode_string(instring: &str) -> String {
    let mut outstring = String::from(instring.len().to_string());
    outstring.push(':');
    outstring.push_str(instring);
    return outstring;
}

fn encode_int(int: i64) -> String {
    let mut outstring = String::from("i");
    outstring.push_str(int.to_string().as_str());
    outstring.push('e');
    return outstring;
}

fn encode_dic(hm: &HashMap<String, BencodeT>) -> String {
    let mut outstring = String::from("d");
    let ordered: BTreeMap<_, _> = hm.iter().collect();
    for (key, value) in ordered.iter() {
        outstring.push_str(encode_string(key).as_str());
        outstring.push_str(encode(value).as_str());
    }
    outstring.push('e');
    return outstring;
}

fn encode_list(vec: &Vec<BencodeT>) -> String {
    let mut outstring = String::from("l");
    for elem in vec {
        outstring.push_str(encode(elem).as_str());
    }
    outstring.push('e');
    return outstring;
}

#[cfg(test)]
mod tests {

    use bittorrent::bencoder::*;
    use std::collections::{HashMap};
    use bittorrent::BencodeT;
    use bittorrent::utils::{create_ints, create_strings};

    #[test]
    fn strings() {
        let (strings, bstrings) = create_strings();
        for (bstring, string) in bstrings.iter().zip(strings) {
            assert_eq!(string, encode(bstring));
        }
    }

    #[test]
    fn ints() {
        let (strings, ints) = create_ints();
        for (int, string) in ints.iter().zip(strings) {
            assert_eq!(string, encode(int));
        }
    }

    #[test]
    fn lists() {
        let (sstrings, sbencoded) = create_strings();
        let (istrings, ibencoded) = create_ints();
        let list = BencodeT::List(sbencoded.clone());
        assert_eq!(encode_list(&sbencoded), encode(&list));

        let string = "li1ei2ei3ee";
        let list = BencodeT::List(
            vec![1, 2, 3]
                .into_iter()
                .map(|i| BencodeT::Integer(i))
                .collect(),
        );
        assert_eq!(string, encode(&list));
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
        assert_eq!(encode_dic(&hm), encode(&bhm));

        let string = "d1:gd1:ei8e1:fi9eee";
        let mut dic1 = HashMap::new();
        dic1.insert(String::from("e"), BencodeT::Integer(8));
        dic1.insert(String::from("f"), BencodeT::Integer(9));
        let mut dic2 = HashMap::new();
        dic2.insert(String::from("g"), BencodeT::Dictionary(dic1));
        assert_eq!(string, encode(&BencodeT::Dictionary(dic2)));
    }
}
