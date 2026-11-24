//! Decodes instructions one after another from an address, without any of
//! the checks a full disassembly makes. For investigating code the
//! disassembler rejects. Usage: `probe <game.hxg> <hex address> [count]`

use hexicon_bytecode::{MethodHeader, decode};
use hexicon_image::Image;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = std::fs::read(&args[0]).expect("cannot read image");
    let image = Image::parse(&data).expect("cannot parse image");
    let address = u32::from_str_radix(args[1].trim_start_matches("0x"), 16).expect("bad address");
    let count: usize = args.get(2).map_or(40, |n| n.parse().expect("bad count"));
    let page = image
        .code_pool
        .at(address)
        .expect("address is not in the code pool");
    println!("{:?}", MethodHeader::parse(page));
    println!("{:02x?}", &page[..page.len().min(16)]);
    let mut offset = u32::from(image.entry_point.method_header_size);
    for _ in 0..count {
        match decode(page, offset) {
            Ok(i) => {
                println!(
                    "{:04x}  {:<14} {:?} {:?}",
                    i.offset,
                    i.opcode.name(),
                    i.operands,
                    i.payload
                );
                offset += i.len;
            }
            Err(error) => {
                println!("{error}");
                break;
            }
        }
    }
}
