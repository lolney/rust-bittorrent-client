use std::vec::Vec;

/*
Does not work - parsing expression grammar is unable to express
the grammar for strings
*/
/*
peg! bencode(r#"
use bittorrent::bencoder::BencodeT;
use bittorrent::bencoder::Kvpair;
use std::str::FromStr;

#[pub]
expressions -> Vec<BencodeT>
	= expression+

#[pub]
expression -> BencodeT
	= list/dictionary
	/ i:integer { BencodeT::Integer{v : i} }
	/ s:string { BencodeT::String{v : s} }

string -> String
	= ":" strlen:uint s:.+ 
	{?
		let floatl = (strlen as f64) + 1.0;
		let numlen = floatl.log10().ceil() as usize;

		if strlen != (pos - start_pos - numlen - 1){
			Err("Reported length does not match string length")
		}
		else{
			let start = start_pos + 1 + numlen;
			Ok(match_str[start .. start+strlen].to_string())
		}
	}

integer -> i64
	= "i" "-"?[0-9]+ "e" {match_str[start_pos+1 .. pos].parse().unwrap()}

uint -> usize
	= [1-9][0-9]* {usize::from_str(match_str).unwrap()}

list -> BencodeT
	= "l" l:(expression+) "e" { BencodeT::List{v : l} }

dictionary -> BencodeT
	= "d" d:dictionary_elem+ "e" { BencodeT::Dictionary{v : d} }

dictionary_elem -> Kvpair
	= k:string v:expression {Kvpair{key:k, value:v}}
"#);
 */

 /*
#[test]
fn strings() {
	assert_eq!(bencode::expression(":1p"),
	 Ok(BencodeT::String{v :String::from("p")}));
	assert_eq!(bencode::expression(":8aodfjdoi"),
	 Ok(BencodeT::String{v :String::from("aodfjdoi")}));

	assert_eq!(bencode::expressions(":1p:2d::4ffef"),
	 Ok(vec!(
	 	BencodeT::String{v :String::from("p")},
	 	BencodeT::String{v :String::from("d:")},
	 	BencodeT::String{v :String::from("ffef")})));

	assert!(bencode::expression(":11aodfjdoi").is_err());
}

#[test]
fn dictionaries() {
	assert_eq!(bencode::expression("d:1o:1oe"),
		Ok(BencodeT::Dictionary{
			v : vec!(
				Kvpair{key: String::from("o"),
					value: BencodeT::String{v :String::from("o")}})
		}));
}

#[test]
fn lists() {
	assert_eq!(bencode::expression("l:1o:1oe"),
		Ok(BencodeT::List{
			v : vec!(BencodeT::String{v :String::from("o")},
						BencodeT::String{v :String::from("o")})
		}));
}
 */