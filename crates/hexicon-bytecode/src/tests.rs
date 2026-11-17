use hexicon_image::Value;

use crate::opcode::{Layout, OperandKind};
use crate::*;

#[test]
fn opcode_table_is_consistent() {
    // Every opcode maps back to itself, and names are unique.
    let mut names = std::collections::HashSet::new();
    for &opcode in Opcode::ALL {
        assert_eq!(Opcode::from_byte(opcode as u8), Some(opcode));
        assert!(
            names.insert(opcode.name()),
            "duplicate name {}",
            opcode.name()
        );
    }
    assert_eq!(Opcode::ALL.len(), 166);
    assert_eq!(Opcode::from_byte(0x00), None);
    assert_eq!(Opcode::from_byte(0x53), None);
    assert_eq!(Opcode::from_byte(0xff), None);
}

#[test]
fn fixed_lengths_match_the_reference_table() {
    // Total instruction lengths, including the opcode byte, spot-checked
    // against the reference interpreter's own length table (vmop.cpp).
    let len = |opcode: Opcode| match opcode.layout() {
        Layout::Fixed(kinds) => 1 + kinds.iter().map(|k| k.size()).sum::<usize>(),
        _ => 0,
    };
    assert_eq!(len(Opcode::Push0), 1);
    assert_eq!(len(Opcode::PushInt8), 2);
    assert_eq!(len(Opcode::PushInt), 5);
    assert_eq!(len(Opcode::PushPropId), 3);
    assert_eq!(len(Opcode::PushBifPtr), 5);
    assert_eq!(len(Opcode::NamedArgPtr), 4);
    assert_eq!(len(Opcode::Call), 6);
    assert_eq!(len(Opcode::ObjCallProp), 8);
    assert_eq!(len(Opcode::CallPropLcl1), 5);
    assert_eq!(len(Opcode::ExpInherit), 8);
    assert_eq!(len(Opcode::PtrExpInherit), 6);
    assert_eq!(len(Opcode::IterNext), 5);
    assert_eq!(len(Opcode::Builtin2), 5);
    assert_eq!(len(Opcode::New2), 5);
    assert_eq!(len(Opcode::AddILcl4), 7);
    assert_eq!(len(Opcode::ObjSetProp), 7);
    assert_eq!(OperandKind::Branch.size(), 2);
}

#[test]
fn decodes_fixed_operands() {
    let code = [0x03, 0xfe];
    let i = decode(&code, 0).unwrap();
    assert_eq!(i.opcode, Opcode::PushInt8);
    assert_eq!(i.len, 2);
    assert_eq!(i.operands, [Operand::Signed(-2)]);

    let code = [0x00, 0x67, 2, 0x78, 0x56, 0x34, 0x12, 0x34, 0x12];
    let i = decode(&code, 1).unwrap();
    assert_eq!(i.opcode, Opcode::ObjCallProp);
    assert_eq!(i.len, 8);
    assert_eq!(
        i.operands,
        [
            Operand::Argc(2),
            Operand::Object(0x1234_5678),
            Operand::Property(0x1234)
        ]
    );
}

#[test]
fn branches_are_relative_to_the_operand() {
    // jmp at offset 4 with offset +3: the operand is at 5, so the target is 8.
    let code = [0, 0, 0, 0, 0x91, 0x03, 0x00];
    assert_eq!(decode(&code, 4).unwrap().operands, [Operand::Branch(8)]);
    // A backward branch: operand at 5, offset -5.
    let code = [0, 0, 0, 0, 0x92, 0xfb, 0xff];
    assert_eq!(decode(&code, 4).unwrap().branch_targets(), [0]);
}

#[test]
fn decodes_inline_strings() {
    let code = [0x0c, 3, 0, b'a', b'b', b'c', 0x54];
    let i = decode(&code, 0).unwrap();
    assert_eq!(i.len, 6);
    assert_eq!(i.payload, Payload::InlineString(b"abc".to_vec()));
}

#[test]
fn decodes_switch_tables() {
    let code = [
        0x90, 2, 0, // two cases
        7, 5, 0, 0, 0, 0x10, 0x00, // int 5 -> operand at 8, +0x10
        1, 0, 0, 0, 0, 0xf0, 0xff, // nil -> operand at 15, -0x10
        0x02, 0x00, // default -> operand at 17, +2
    ];
    let i = decode(&code, 0).unwrap();
    assert_eq!(i.len, 19);
    assert_eq!(
        i.payload,
        Payload::Switch {
            cases: vec![(Value::Int(5), 0x18), (Value::Nil, -1)],
            default: 19,
        }
    );
    assert_eq!(i.branch_targets(), [0x18, -1, 19]);
}

#[test]
fn decodes_named_argument_tables() {
    // Table: count 2; index offsets 6, 9, 13 from the start of the index;
    // names "foo" and "quux".
    let mut code = vec![0x57, 15, 0, 2, 0, 6, 0, 9, 0, 13, 0];
    code.extend_from_slice(b"fooquux");
    let i = decode(&code, 0).unwrap();
    assert_eq!(i.len as usize, code.len());
    assert_eq!(
        i.payload,
        Payload::NamedArgs(vec!["foo".into(), "quux".into()])
    );
}

#[test]
fn reports_bad_code() {
    assert_eq!(
        decode(&[0x53], 0).unwrap_err(),
        DecodeError::InvalidOpcode {
            offset: 0,
            byte: 0x53
        }
    );
    assert_eq!(
        decode(&[0x04, 1, 2], 0).unwrap_err(),
        DecodeError::Truncated {
            offset: 0,
            opcode: Opcode::PushInt
        }
    );
    assert!(decode(&[], 0).is_err());
    assert!(decode(&[0x57, 2, 0, 9, 0], 0).is_err());
    // No input may panic, whatever it holds.
    for byte in 0..=255u8 {
        for len in 0..6 {
            let _ = decode(&vec![byte; len], 0);
        }
    }
}

#[test]
fn parses_method_headers() {
    let header = MethodHeader::parse(&[0x82, 1, 3, 0, 9, 0, 0x40, 0, 0, 0]).unwrap();
    assert_eq!(header.param_count, 2);
    assert!(header.varargs);
    assert_eq!(header.optional_param_count, 1);
    assert_eq!(header.local_count, 3);
    assert_eq!(header.stack_depth, 9);
    assert_eq!(header.exception_table_offset, 0x40);
    assert_eq!(header.debug_records_offset, 0);
    assert!(MethodHeader::parse(&[0; 9]).is_none());
}
