use bittorrent::BencodeT;

pub fn parse_i64(instring: String) -> Result<i64, String> {
    let mut string = instring;
    let mut negative = false;
    if string.chars().next().unwrap() == '-' {
        string = string.split_off(1);
        negative = true;
    }

    if string.len() > 19 {
        return Err(format!("Overflow: {:?}", string));
    } else if string.len() == 19 {
        for (d1, d2) in String::from("9223372036854775807")
            .chars()
            .zip(string.chars())
        {
            if d1 > d2 {
                return Err(format!("Overflow: {:?}", string));
            }
            if d1 < d2 {
                break;
            }
        }
    }
    let mut total: i64 = 0;
    let ten: i64 = 10;
    for (i, item) in string.chars().rev().enumerate() {
        let digit: i64 = item.to_digit(10).unwrap() as i64;
        let j: u32 = i as u32;
        total = digit * ten.pow(j) + total;
    }

    if negative {
        total = total * -1
    }
    Ok(total)
}

pub fn create_strings() -> (Vec<&'static str>, Vec<BencodeT>) {
    let strings = vec![
    "5:abcde",
    "26:abcdefghijklmnopQRSTUVWxyz",
    "285:Like vectors, HashMaps are growable, but HashMaps can also shrink themselves when they have excess space. You can create a HashMap with a certain starting capacity using HashMap::with_capacity(uint), or use HashMap::new() to get a HashMap with a default initial capacity (recommended)."];
    let bstrings = vec![
        BencodeT::String("abcde".to_string()),
        BencodeT::String("abcdefghijklmnopQRSTUVWxyz".to_string()),
        BencodeT::String("Like vectors, HashMaps are growable, but HashMaps can also shrink themselves when they have excess space. You can create a HashMap with a certain starting capacity using HashMap::with_capacity(uint), or use HashMap::new() to get a HashMap with a default initial capacity (recommended).".to_string())];
    (strings, bstrings)
}

pub fn create_ints() -> (Vec<&'static str>, Vec<BencodeT>) {
    let strings = vec![
        "i9223372036854775807e",
        "i3e",
        "i-3e",
        "i0e",
        "i-9223372036854775807e",
    ];
    let bints = vec![
        BencodeT::Integer(9223372036854775807),
        BencodeT::Integer(3),
        BencodeT::Integer(-3),
        BencodeT::Integer(0),
        BencodeT::Integer(-9223372036854775807),
    ];
    (strings, bints)
}
