/*use nom::*;

use std::result::Result;
use std::str;
use std::str::FromStr;

/*
*	Take bencoded string as input and output its representation as an IResult
*/
fn parse(o : &[u8]) -> Result<u64, u64> {
	match str::from_utf8(o) {
		Ok(v) => Ok(90u64),
		Err(v) => Ok(90u64),
	}
}

named!(string_tag(&[u8]) -> u64,
	map_res!(
		preceded!(tag!(":"),
			take_while!(is_digit)),
	parse));

#[test]
fn string_tag_test() {
	assert_eq!(string_tag(&b":100"[..]), IResult::Done(&b""[..], 78));
}

fn my_function(input: &[u8]) -> IResult<&[u8], &[u8]> {
  tag!(input, "1")
}

// fn parser(input: I) -> IResult<I, O>; <- parsers are of this form 
// Combinators take parsers as input
#[test]
fn test() {
    assert_eq!(my_function(&b"10000"[..]),IResult::Done(&b"0000"[..],&b"1"[..]));;
}
*/
