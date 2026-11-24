use super::*;

/// Builds image files block by block.
struct Builder {
    data: Vec<u8>,
}

impl Builder {
    fn new() -> Self {
        let mut data = SIGNATURE.to_vec();
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&[0; 28]);
        data.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        data.extend_from_slice(b"Sun Aug 01 17:05:20 1999");
        Self { data }
    }

    fn block(mut self, kind: &[u8; 4], flags: u16, body: &[u8]) -> Self {
        self.data.extend_from_slice(kind);
        self.data
            .extend_from_slice(&(body.len() as u32).to_le_bytes());
        self.data.extend_from_slice(&flags.to_le_bytes());
        self.data.extend_from_slice(body);
        self
    }

    fn entry_point(self) -> Self {
        let mut body = 0x1234u32.to_le_bytes().to_vec();
        for size in [10u16, 10, 10, 6, 6, 2, 8] {
            body.extend_from_slice(&size.to_le_bytes());
        }
        self.block(b"ENTP", 1, &body)
    }

    fn metaclasses(self) -> Self {
        let name = b"hexicon-object/030005";
        let mut body = 1u16.to_le_bytes().to_vec();
        let entry_size = 2 + 1 + name.len() + 2 + 2 + 4;
        body.extend_from_slice(&(entry_size as u16).to_le_bytes());
        body.push(name.len() as u8);
        body.extend_from_slice(name);
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&7u16.to_le_bytes());
        body.extend_from_slice(&9u16.to_le_bytes());
        self.block(b"MCLD", 1, &body)
    }

    fn function_sets(self) -> Self {
        let mut body = 1u16.to_le_bytes().to_vec();
        let name = b"hexicon-vm/010006";
        body.push(name.len() as u8);
        body.extend_from_slice(name);
        self.block(b"FNSD", 1, &body)
    }

    /// The blocks every program image must have.
    fn required(self) -> Self {
        self.entry_point().metaclasses().function_sets()
    }

    fn finish(self) -> Vec<u8> {
        self.block(b"EOF ", 1, &[]).data
    }
}

#[test]
fn minimal_image() {
    let image = Image::parse(&Builder::new().required().finish()).unwrap();
    assert_eq!(image.header.version, 2);
    assert_eq!(image.header.build_configuration, [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(image.header.timestamp, "Sun Aug 01 17:05:20 1999");
    assert_eq!(image.entry_point.code_offset, 0x1234);
    assert_eq!(image.entry_point.method_header_size, 10);
    assert_eq!(image.entry_point.debug_frame_header_size, 8);
    assert_eq!(image.metaclasses.len(), 1);
    assert_eq!(image.metaclasses[0].base_name(), "hexicon-object");
    assert_eq!(image.metaclasses[0].version(), Some("030005"));
    assert_eq!(image.metaclasses[0].properties, [7, 9]);
    assert_eq!(image.function_sets, ["hexicon-vm/010006"]);
    assert_eq!(image.blocks.len(), 4);
    assert_eq!(image.blocks[3].kind, BlockKind::EOF);
}

#[test]
fn data_after_eof_is_ignored() {
    let mut data = Builder::new().required().finish();
    data.extend_from_slice(b"trailing data that is not part of the image");
    assert!(Image::parse(&data).is_ok());
}

#[test]
fn rejects_non_images() {
    assert_eq!(Image::parse(b"").unwrap_err(), ImageError::NotAnImage);
    assert_eq!(
        Image::parse(b"TADS2 bin\x0a\x0d\x1a").unwrap_err(),
        ImageError::NotAnImage
    );
}

#[test]
fn rejects_newer_versions() {
    let mut data = Builder::new().required().finish();
    data[11] = 3;
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::UnsupportedVersion(3)
    );
}

#[test]
fn reports_truncation() {
    let data = Builder::new().required().finish();
    // Cut the file at every length; none may panic, and all must fail.
    for len in 0..data.len() {
        assert!(Image::parse(&data[..len]).is_err(), "length {len}");
    }
}

#[test]
fn reports_missing_required_blocks() {
    let data = Builder::new().metaclasses().function_sets().finish();
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::MissingBlock(BlockKind::ENTP)
    );
    let data = Builder::new().entry_point().function_sets().finish();
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::MissingBlock(BlockKind::MCLD)
    );
    let data = Builder::new().entry_point().metaclasses().finish();
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::MissingBlock(BlockKind::FNSD)
    );
}

#[test]
fn reports_duplicate_entry_point() {
    let data = Builder::new().required().entry_point().finish();
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::DuplicateBlock(BlockKind::ENTP)
    );
}

#[test]
fn unknown_blocks() {
    let data = Builder::new()
        .required()
        .block(b"XYZW", 0, b"ignored")
        .finish();
    let image = Image::parse(&data).unwrap();
    assert_eq!(image.blocks[3].kind, BlockKind(*b"XYZW"));
    assert!(!image.blocks[3].is_mandatory());

    let data = Builder::new()
        .required()
        .block(b"XYZW", 1, b"needed")
        .finish();
    assert!(matches!(
        Image::parse(&data).unwrap_err(),
        ImageError::UnknownMandatoryBlock {
            kind: BlockKind([b'X', b'Y', b'Z', b'W']),
            ..
        }
    ));
}

#[test]
fn entry_point_without_frame_size() {
    let mut body = 0u32.to_le_bytes().to_vec();
    for size in [10u16, 10, 10, 6, 6, 1] {
        body.extend_from_slice(&size.to_le_bytes());
    }
    let data = Builder::new()
        .block(b"ENTP", 1, &body)
        .metaclasses()
        .function_sets()
        .finish();
    assert_eq!(
        Image::parse(&data)
            .unwrap()
            .entry_point
            .debug_frame_header_size,
        4
    );
}

fn pool_definition(id: u16, pages: u32, page_size: u32) -> Vec<u8> {
    let mut body = id.to_le_bytes().to_vec();
    body.extend_from_slice(&pages.to_le_bytes());
    body.extend_from_slice(&page_size.to_le_bytes());
    body
}

fn pool_page(id: u16, index: u32, mask: u8, data: &[u8]) -> Vec<u8> {
    let mut body = id.to_le_bytes().to_vec();
    body.extend_from_slice(&index.to_le_bytes());
    body.push(mask);
    body.extend(data.iter().map(|b| b ^ mask));
    body
}

#[test]
fn pools() {
    let data = Builder::new()
        .required()
        .block(b"CPDF", 1, &pool_definition(POOL_CONSTANT, 3, 8))
        .block(b"CPPG", 1, &pool_page(POOL_CONSTANT, 2, 0x5a, b"late"))
        .block(b"CPPG", 1, &pool_page(POOL_CONSTANT, 0, 0, b"12345678"))
        .finish();
    let image = Image::parse(&data).unwrap();
    let pool = &image.constant_pool;
    assert_eq!(pool.page_size(), 8);
    assert_eq!(pool.page_count(), 3);
    assert_eq!(pool.loaded_page_count(), 2);
    assert_eq!(pool.data_len(), 12);
    assert_eq!(pool.at(0), Some(&b"12345678"[..]));
    assert_eq!(pool.at(5), Some(&b"678"[..]));
    assert_eq!(pool.at(8), None, "page 1 is not in the image");
    assert_eq!(pool.at(17), Some(&b"ate"[..]), "page data is unmasked");
    assert_eq!(pool.at(21), None, "past the end of a short page");
    assert_eq!(pool.at(24), None);
    assert_eq!(image.code_pool.page_count(), 0);
    assert_eq!(image.code_pool.at(0), None);
}

#[test]
fn pool_errors() {
    let parse = |builder: Builder| Image::parse(&builder.finish()).unwrap_err();
    assert_eq!(
        parse(
            Builder::new()
                .required()
                .block(b"CPPG", 1, &pool_page(POOL_CODE, 0, 0, b"x"))
        ),
        ImageError::PageBeforePoolDefinition(POOL_CODE)
    );
    assert_eq!(
        parse(
            Builder::new()
                .required()
                .block(b"CPDF", 1, &pool_definition(3, 1, 8))
        ),
        ImageError::UnknownPool(3)
    );
    let defined = || {
        Builder::new()
            .required()
            .block(b"CPDF", 1, &pool_definition(POOL_CODE, 1, 4))
    };
    assert_eq!(
        parse(defined().block(b"CPDF", 1, &pool_definition(POOL_CODE, 1, 4))),
        ImageError::DuplicatePool(POOL_CODE)
    );
    assert_eq!(
        parse(defined().block(b"CPPG", 1, &pool_page(POOL_CODE, 1, 0, b"x"))),
        ImageError::PageIndexOutOfRange {
            pool: POOL_CODE,
            index: 1
        }
    );
    assert_eq!(
        parse(defined().block(b"CPPG", 1, &pool_page(POOL_CODE, 0, 0, b"12345"))),
        ImageError::PageTooLarge {
            pool: POOL_CODE,
            index: 0
        }
    );
    let page = pool_page(POOL_CODE, 0, 0, b"x");
    assert_eq!(
        parse(defined().block(b"CPPG", 1, &page).block(b"CPPG", 1, &page)),
        ImageError::DuplicatePage {
            pool: POOL_CODE,
            index: 0
        }
    );
}

fn object_block(metaclass: u16, flags: u16, objects: &[(u32, &[u8])]) -> Vec<u8> {
    let mut body = (objects.len() as u16).to_le_bytes().to_vec();
    body.extend_from_slice(&metaclass.to_le_bytes());
    body.extend_from_slice(&flags.to_le_bytes());
    for (id, data) in objects {
        body.extend_from_slice(&id.to_le_bytes());
        if flags & 1 != 0 {
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        } else {
            body.extend_from_slice(&(data.len() as u16).to_le_bytes());
        }
        body.extend_from_slice(data);
    }
    body
}

#[test]
fn static_objects() {
    let data = Builder::new()
        .required()
        .block(b"OBJS", 1, &object_block(0, 0, &[(5, b"abc"), (9, b"")]))
        .block(
            b"OBJS",
            1,
            &object_block(0, 3, &[(12, b"large and transient")]),
        )
        .finish();
    let image = Image::parse(&data).unwrap();
    assert_eq!(image.object_count(), 3);
    let first = &image.object_blocks[0];
    assert!(!first.transient);
    assert_eq!(first.objects[0].id, 5);
    assert_eq!(&*first.objects[0].data, b"abc");
    assert_eq!(first.objects[1].id, 9);
    assert!(first.objects[1].data.is_empty());
    let second = &image.object_blocks[1];
    assert!(second.transient);
    assert_eq!(&*second.objects[0].data, b"large and transient");
    assert_eq!(image.metaclass_of(second).name, "hexicon-object/030005");
}

#[test]
fn static_object_errors() {
    let objects = object_block(0, 0, &[(5, b"abc")]);
    let data = Builder::new()
        .entry_point()
        .block(b"OBJS", 1, &objects)
        .metaclasses()
        .function_sets()
        .finish();
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::ObjectsBeforeMetaclasses
    );

    let data = Builder::new()
        .required()
        .block(b"OBJS", 1, &object_block(1, 0, &[]))
        .finish();
    assert_eq!(
        Image::parse(&data).unwrap_err(),
        ImageError::MetaclassIndexOutOfRange(1)
    );

    let mut short = objects.clone();
    short.pop();
    let data = Builder::new().required().block(b"OBJS", 1, &short).finish();
    assert!(matches!(
        Image::parse(&data).unwrap_err(),
        ImageError::TruncatedBlock {
            kind: BlockKind::OBJS,
            ..
        }
    ));
}

#[test]
fn symbols_and_values() {
    let mut body = 3u16.to_le_bytes().to_vec();
    for (holder, name) in [
        ([6u8, 0x34, 0x12, 0, 0], &b"Constructor"[..]),
        ([5, 0x78, 0x56, 0x34, 0x12], b"RuntimeError"),
        ([16, 3, 0, 1, 0], b"someFunction"),
    ] {
        body.extend_from_slice(&holder);
        body.push(name.len() as u8);
        body.extend_from_slice(name);
    }
    let data = Builder::new().required().block(b"SYMD", 1, &body).finish();
    let image = Image::parse(&data).unwrap();
    assert_eq!(image.symbol("Constructor"), Some(Value::Property(0x1234)));
    assert_eq!(
        image.symbol("RuntimeError"),
        Some(Value::Object(0x1234_5678))
    );
    assert_eq!(
        image.symbol("someFunction"),
        Some(Value::IntrinsicFunction { set: 1, index: 3 })
    );
    assert_eq!(image.symbol("missing"), None);

    assert_eq!(Value::decode([1, 9, 9, 9, 9]), Value::Nil);
    assert_eq!(Value::decode([7, 0xff, 0xff, 0xff, 0xff]), Value::Int(-1));
    assert_eq!(
        Value::decode([99, 1, 2, 3, 4]),
        Value::Other {
            type_code: 99,
            raw: [1, 2, 3, 4]
        }
    );
}

#[test]
fn static_initializer_list() {
    let mut body = 16u32.to_le_bytes().to_vec();
    body.extend_from_slice(&0x8000u32.to_le_bytes());
    body.extend_from_slice(&2u32.to_le_bytes());
    body.extend_from_slice(&[0; 4]); // a header field from a later version
    for (object, property) in [(10u32, 3u16), (11, 4)] {
        body.extend_from_slice(&object.to_le_bytes());
        body.extend_from_slice(&property.to_le_bytes());
    }
    let data = Builder::new().required().block(b"SINI", 1, &body).finish();
    let list = Image::parse(&data).unwrap().static_initializers.unwrap();
    assert_eq!(list.static_code_offset, 0x8000);
    assert_eq!(
        list.initializers,
        [
            StaticInitializer {
                object: 10,
                property: 3
            },
            StaticInitializer {
                object: 11,
                property: 4
            }
        ]
    );
}

#[test]
fn embedded_resources() {
    let name: Vec<u8> = b"cover.jpg".iter().map(|b| b ^ 0xff).collect();
    let toc_size = 2 + 9 + name.len();
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend_from_slice(&(toc_size as u32).to_le_bytes());
    body.extend_from_slice(&4u32.to_le_bytes());
    body.push(name.len() as u8);
    body.extend_from_slice(&name);
    body.extend_from_slice(b"JPEG");
    let data = Builder::new().required().block(b"MRES", 0, &body).finish();
    let image = Image::parse(&data).unwrap();
    let resource = &image.resources[0];
    assert_eq!(resource.name, "cover.jpg");
    assert_eq!(
        &data[resource.offset..resource.offset + resource.size as usize],
        b"JPEG"
    );
}

#[test]
fn hostile_counts_do_not_allocate() {
    // Each of these claims far more entries than the block can hold.
    for kind in [b"GSYM", b"MHLS", b"MACR"] {
        let data = Builder::new()
            .required()
            .block(kind, 0, &u32::MAX.to_le_bytes())
            .finish();
        assert!(Image::parse(&data).is_err(), "{}", BlockKind(*kind));
    }
    let data = Builder::new()
        .required()
        .block(b"CPDF", 1, &pool_definition(POOL_CODE, u32::MAX, 4096))
        .finish();
    assert!(Image::parse(&data).is_err());
}

#[test]
fn reads_only_hexicons_signature() {
    let data = Builder::new().entry_point().finish();
    assert!(parse_header(&data).is_ok());
    for other in [b"T3-image\r\n\x1a".as_slice(), b"XX-image\r\n\x1a"] {
        let mut data = Builder::new().entry_point().finish();
        data[..other.len()].copy_from_slice(other);
        assert!(matches!(parse_header(&data), Err(ImageError::NotAnImage)));
    }
}
