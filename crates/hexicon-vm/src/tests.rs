//! Unit tests. The programs here are assembled by hand into small images,
//! so the tests need nothing outside this repository.

use hexicon_bytecode::Opcode as Op;
use hexicon_host::BufferHost;
use hexicon_image::Image;

use crate::console::{Formatter, Newline};
use crate::error::{ErrParam, VmError, code, format_message};
use crate::object::{Object, ObjectTable};
use crate::tads::{self, TadsObject};
use crate::value::{ObjId, Value, type_code};
use crate::vm::Vm;

// ---- building images ----

/// Index of each intrinsic class in the test images' dependency list.
const META_TADS_OBJECT: u8 = 0;

/// Index of each function set in the test images' dependency list.
const SET_TADS_GEN: u8 = 1;
const SET_TADS_IO: u8 = 2;

/// Property IDs the test images use.
const PROP_CONSTRUCT: u16 = 1;
const PROP_MESSAGE: u16 = 2;
const PROP_NAME: u16 = 10;
const PROP_GREET: u16 = 11;
const PROP_LENGTH: u16 = 20;

/// Assembles a code pool. Functions are appended one after another.
#[derive(Default)]
struct Code {
    bytes: Vec<u8>,
}

impl Code {
    /// Begin a function and return its address.
    fn function(&mut self, params: u8, locals: u16) -> u32 {
        let address = self.bytes.len() as u32;
        self.bytes.push(params);
        self.bytes.push(0);
        self.bytes.extend_from_slice(&locals.to_le_bytes());
        // Stack depth, exception table offset, debug records offset.
        self.bytes.extend_from_slice(&[16, 0, 0, 0, 0, 0]);
        address
    }

    fn op(&mut self, opcode: Op) -> &mut Self {
        self.bytes.push(opcode as u8);
        self
    }

    fn u8(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A branch operand to an absolute address.
    fn branch_to(&mut self, target: usize) -> &mut Self {
        let offset = target as i64 - self.bytes.len() as i64;
        self.u16(offset as i16 as u16)
    }

    /// A branch operand to be filled in later. Returns its position.
    fn branch_later(&mut self) -> usize {
        let at = self.bytes.len();
        self.u16(0);
        at
    }

    /// Point an earlier branch operand at the current position.
    fn land(&mut self, operand: usize) {
        let offset = (self.bytes.len() as i64 - operand as i64) as i16;
        self.bytes[operand..operand + 2].copy_from_slice(&offset.to_le_bytes());
    }

    fn here(&self) -> usize {
        self.bytes.len()
    }

    /// Display the value on top of the stack with `tadsSay`.
    fn say(&mut self) -> &mut Self {
        self.op(Op::Builtin1).u8(1).u8(0).u8(SET_TADS_IO)
    }

    /// Push a string constant and display it.
    fn say_str(&mut self, offset: u32) -> &mut Self {
        self.op(Op::PushStr).u32(offset).say()
    }
}

/// Assembles a constant pool.
#[derive(Default)]
struct Consts {
    bytes: Vec<u8>,
}

impl Consts {
    fn string(&mut self, text: &str) -> u32 {
        let offset = self.bytes.len() as u32;
        self.bytes
            .extend_from_slice(&(text.len() as u16).to_le_bytes());
        self.bytes.extend_from_slice(text.as_bytes());
        offset
    }

    fn list(&mut self, items: &[[u8; 5]]) -> u32 {
        let offset = self.bytes.len() as u32;
        self.bytes
            .extend_from_slice(&(items.len() as u16).to_le_bytes());
        for item in items {
            self.bytes.extend_from_slice(item);
        }
        offset
    }
}

/// A value in the five-byte form images use.
fn holder(type_code: i32, value: u32) -> [u8; 5] {
    let v = value.to_le_bytes();
    [type_code as u8, v[0], v[1], v[2], v[3]]
}

/// The image data of a program-defined object.
fn tads_object_data(superclasses: &[u32], is_class: bool, props: &[(u16, [u8; 5])]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&(superclasses.len() as u16).to_le_bytes());
    data.extend_from_slice(&(props.len() as u16).to_le_bytes());
    data.extend_from_slice(&u16::from(is_class).to_le_bytes());
    for superclass in superclasses {
        data.extend_from_slice(&superclass.to_le_bytes());
    }
    for (prop, value) in props {
        data.extend_from_slice(&prop.to_le_bytes());
        data.extend_from_slice(value);
    }
    data
}

/// Assembles an image file.
#[derive(Default)]
struct ImageBuilder {
    objects: Vec<(u32, Vec<u8>)>,
    symbols: Vec<(String, [u8; 5])>,
}

impl ImageBuilder {
    fn object(mut self, id: u32, data: Vec<u8>) -> Self {
        self.objects.push((id, data));
        self
    }

    fn build(self, entry: u32, code: &Code, consts: &Consts) -> Image {
        fn block(data: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
            data.extend_from_slice(kind);
            data.extend_from_slice(&(body.len() as u32).to_le_bytes());
            data.extend_from_slice(&1u16.to_le_bytes());
            data.extend_from_slice(body);
        }
        fn pool(data: &mut Vec<u8>, id: u16, contents: &[u8]) {
            let mut body = id.to_le_bytes().to_vec();
            body.extend_from_slice(&1u32.to_le_bytes());
            body.extend_from_slice(&4096u32.to_le_bytes());
            block(data, b"CPDF", &body);
            let mut body = id.to_le_bytes().to_vec();
            body.extend_from_slice(&0u32.to_le_bytes());
            body.push(0);
            body.extend_from_slice(contents);
            block(data, b"CPPG", &body);
        }

        let mut data = hexicon_image::SIGNATURE.to_vec();
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&[0; 32]);
        data.extend_from_slice(b"Sun Aug 01 17:05:20 1999");

        let mut body = entry.to_le_bytes().to_vec();
        for size in [10u16, 10, 10, 6, 6, 2, 8] {
            body.extend_from_slice(&size.to_le_bytes());
        }
        block(&mut data, b"ENTP", &body);

        // The string class has one method, `length`.
        let metaclasses: [(&str, &[u16]); 4] = [
            ("tads-object/030005", &[]),
            ("string/030008", &[PROP_LENGTH]),
            ("list/030008", &[]),
            ("intrinsic-class/030001", &[]),
        ];
        let mut body = (metaclasses.len() as u16).to_le_bytes().to_vec();
        for (name, props) in metaclasses {
            let size = 2 + 1 + name.len() + 2 + 2 + props.len() * 2;
            body.extend_from_slice(&(size as u16).to_le_bytes());
            body.push(name.len() as u8);
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(&(props.len() as u16).to_le_bytes());
            body.extend_from_slice(&2u16.to_le_bytes());
            for prop in props {
                body.extend_from_slice(&prop.to_le_bytes());
            }
        }
        block(&mut data, b"MCLD", &body);

        let sets = ["t3vm/010006", "tads-gen/030008", "tads-io/030007"];
        let mut body = (sets.len() as u16).to_le_bytes().to_vec();
        for name in sets {
            body.push(name.len() as u8);
            body.extend_from_slice(name.as_bytes());
        }
        block(&mut data, b"FNSD", &body);

        pool(&mut data, hexicon_image::POOL_CODE, &code.bytes);
        pool(&mut data, hexicon_image::POOL_CONSTANT, &consts.bytes);

        if !self.objects.is_empty() {
            let mut body = (self.objects.len() as u16).to_le_bytes().to_vec();
            body.extend_from_slice(&u16::from(META_TADS_OBJECT).to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            for (id, object) in &self.objects {
                body.extend_from_slice(&id.to_le_bytes());
                body.extend_from_slice(&(object.len() as u16).to_le_bytes());
                body.extend_from_slice(object);
            }
            block(&mut data, b"OBJS", &body);
        }

        let mut symbols = self.symbols;
        symbols.push((
            "Constructor".to_owned(),
            holder(type_code::PROP, PROP_CONSTRUCT.into()),
        ));
        symbols.push((
            "exceptionMessage".to_owned(),
            holder(type_code::PROP, PROP_MESSAGE.into()),
        ));
        symbols.push(("LastProp".to_owned(), holder(type_code::PROP, 100)));
        let mut body = (symbols.len() as u16).to_le_bytes().to_vec();
        for (name, value) in &symbols {
            body.extend_from_slice(value);
            body.push(name.len() as u8);
            body.extend_from_slice(name.as_bytes());
        }
        block(&mut data, b"SYMD", &body);

        block(&mut data, b"EOF ", &[]);
        Image::parse(&data).expect("the test image is well formed")
    }
}

/// Run an image and return what it displayed and how it ended.
fn run(image: &Image) -> (String, Result<(), String>) {
    let host = BufferHost::new();
    let mut vm = Vm::new(image, Box::new(host.clone())).expect("the image loads");
    let result = vm.run(&[]);
    let result = result.map_err(|error| vm.error_message(&error, true));
    vm.flush_output();
    (host.output(), result)
}

// ---- programs ----

#[test]
fn displays_a_string() {
    let mut consts = Consts::default();
    let hello = consts.string("Hello, world!\n");
    let mut code = Code::default();
    let main = code.function(1, 0);
    code.say_str(hello).op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(run(&image), ("Hello, world!\n".to_owned(), Ok(())));
}

#[test]
fn loops_and_adds() {
    // local sum = 0; for (local i = 1 ; i <= 10 ; ++i) sum += i;
    let consts = Consts::default();
    let mut code = Code::default();
    let main = code.function(1, 2);
    code.op(Op::ZeroLcl1).u8(0).op(Op::OneLcl1).u8(1);
    let top = code.here();
    code.op(Op::GetLcl1)
        .u8(1)
        .op(Op::PushInt8)
        .u8(10)
        .op(Op::Jgt);
    let done = code.branch_later();
    code.op(Op::GetLcl1).u8(1).op(Op::AddToLcl).u16(0);
    code.op(Op::IncLcl).u16(1).op(Op::Jmp).branch_to(top);
    code.land(done);
    code.op(Op::GetLcl1).u8(0).say().op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(run(&image).0, "55");
}

#[test]
fn calls_a_recursive_function() {
    let consts = Consts::default();
    let mut code = Code::default();
    // factorial(n) { return n > 1 ? n * factorial(n - 1) : 1; }
    let factorial = code.function(1, 0);
    code.op(Op::GetArgN0).op(Op::Push1).op(Op::Jle);
    let base_case = code.branch_later();
    code.op(Op::GetArgN0).op(Op::GetArgN0).op(Op::Dec);
    code.op(Op::Call).u8(1).u32(factorial);
    code.op(Op::GetR0).op(Op::Mul).op(Op::RetVal);
    code.land(base_case);
    code.op(Op::Push1).op(Op::RetVal);

    let main = code.function(1, 0);
    code.op(Op::PushInt8).u8(7);
    code.op(Op::Call).u8(1).u32(factorial);
    code.op(Op::GetR0).say().op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(run(&image).0, "5040");
}

#[test]
fn concatenates_strings_and_numbers() {
    let mut consts = Consts::default();
    let text = consts.string("n = ");
    let mut code = Code::default();
    let main = code.function(1, 0);
    code.op(Op::PushStr).u32(text);
    code.op(Op::PushInt8).u8(42).op(Op::Add);
    code.op(Op::PushNil).op(Op::Add);
    code.say().op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(run(&image).0, "n = 42");
}

#[test]
fn indexes_and_compares_lists() {
    let mut consts = Consts::default();
    let list = consts.list(&[
        holder(type_code::INT, 10),
        holder(type_code::INT, 20),
        holder(type_code::INT, 30),
    ]);
    let same = consts.list(&[
        holder(type_code::INT, 10),
        holder(type_code::INT, 20),
        holder(type_code::INT, 30),
    ]);
    let yes = consts.string(" equal");
    let mut code = Code::default();
    let main = code.function(1, 0);
    code.op(Op::PushLst).u32(list).op(Op::IdxInt8).u8(2).say();
    code.op(Op::PushLst).u32(list).op(Op::PushLst).u32(same);
    code.op(Op::Jne);
    let differ = code.branch_later();
    code.say_str(yes);
    code.land(differ);
    code.op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(run(&image).0, "20 equal");
}

#[test]
fn calls_methods_and_inherits() {
    let mut consts = Consts::default();
    let name_base = consts.string("base");
    let name_derived = consts.string("derived");
    let hello = consts.string("hello from ");
    let again = consts.string(" again");
    let mut code = Code::default();

    // Base.greet() { tadsSay('hello from ' + name); }
    let base_greet = code.function(0, 0);
    code.op(Op::PushStr).u32(hello);
    code.op(Op::GetPropSelf)
        .u16(PROP_NAME)
        .op(Op::GetR0)
        .op(Op::Add);
    code.say().op(Op::RetNil);

    // Derived.greet() { inherited(); tadsSay(' again'); }
    let derived_greet = code.function(0, 0);
    code.op(Op::Inherit).u8(0).u16(PROP_GREET);
    code.say_str(again).op(Op::RetNil);

    let main = code.function(1, 0);
    code.op(Op::ObjCallProp).u8(0).u32(2).u16(PROP_GREET);
    code.op(Op::RetNil);

    let image = ImageBuilder::default()
        .object(
            1,
            tads_object_data(
                &[],
                true,
                &[
                    (PROP_NAME, holder(type_code::SSTRING, name_base)),
                    (PROP_GREET, holder(type_code::CODEOFS, base_greet)),
                ],
            ),
        )
        .object(
            2,
            tads_object_data(
                &[1],
                false,
                &[
                    (PROP_NAME, holder(type_code::SSTRING, name_derived)),
                    (PROP_GREET, holder(type_code::CODEOFS, derived_greet)),
                ],
            ),
        )
        .build(main, &code, &consts);
    assert_eq!(run(&image).0, "hello from derived again");
}

#[test]
fn creates_objects_and_runs_constructors() {
    let mut consts = Consts::default();
    let made = consts.string("made ");
    let mut code = Code::default();

    // Thing.construct(n) { name = n; }  Constructors return self.
    let construct = code.function(1, 0);
    code.op(Op::GetArgN0).op(Op::SetPropSelf).u16(PROP_NAME);
    code.op(Op::PushSelf).op(Op::RetVal);

    // local t = new Thing(7); tadsSay('made ' + t.name);
    let main = code.function(1, 1);
    code.op(Op::PushInt8).u8(7).op(Op::PushObj).u32(1);
    code.op(Op::New1).u8(2).u8(META_TADS_OBJECT);
    code.op(Op::SetLcl1R0).u8(0);
    code.op(Op::PushStr).u32(made);
    code.op(Op::GetPropLcl1)
        .u8(0)
        .u16(PROP_NAME)
        .op(Op::GetR0)
        .op(Op::Add);
    code.say().op(Op::RetNil);

    let image = ImageBuilder::default()
        .object(
            1,
            tads_object_data(
                &[],
                true,
                &[(PROP_CONSTRUCT, holder(type_code::CODEOFS, construct))],
            ),
        )
        .build(main, &code, &consts);
    assert_eq!(run(&image).0, "made 7");
}

#[test]
fn undefined_property_is_nil() {
    let mut consts = Consts::default();
    let nil = consts.string("nil");
    let mut code = Code::default();
    let main = code.function(1, 0);
    code.op(Op::ObjGetProp).u32(1).u16(PROP_GREET);
    code.op(Op::GetR0).op(Op::JNotNil);
    let not_nil = code.branch_later();
    code.say_str(nil);
    code.land(not_nil);
    code.op(Op::RetNil);

    let image = ImageBuilder::default()
        .object(1, tads_object_data(&[], false, &[]))
        .build(main, &code, &consts);
    assert_eq!(run(&image).0, "nil");
}

#[test]
fn calls_string_methods_on_constants() {
    let mut consts = Consts::default();
    let text = consts.string("h\u{e9}llo");
    let mut code = Code::default();
    let main = code.function(1, 0);
    code.op(Op::PushStr).u32(text);
    code.op(Op::GetProp).u16(PROP_LENGTH);
    code.op(Op::GetR0).say().op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    // Length is in characters, not bytes.
    assert_eq!(run(&image).0, "5");
}

#[test]
fn catches_a_thrown_exception() {
    let mut consts = Consts::default();
    let caught = consts.string("caught");
    let mut code = Code::default();

    // thrower() { throw object 1; }
    let thrower = code.function(0, 0);
    code.op(Op::PushObj).u32(1).op(Op::Throw);

    // main() { try { thrower(); } catch (object 1 e) { tadsSay('caught'); } }
    let main = code.function(1, 1);
    // The protected range has to include the instruction after the call,
    // because that is where the function is when the exception arrives.
    let try_start = code.here();
    code.op(Op::Call).u8(0).u32(thrower);
    code.op(Op::RetNil);
    let try_end = code.here() - 1;
    let handler = code.here();
    code.op(Op::SetLcl1).u8(0);
    code.say_str(caught).op(Op::RetNil);
    // The exception table: one handler.
    let table = code.here();
    code.u16(1);
    code.u16((try_start - main as usize) as u16);
    code.u16((try_end - main as usize) as u16);
    code.u32(1);
    code.u16((handler - main as usize) as u16);
    let table_offset = (table - main as usize) as u16;
    code.bytes[main as usize + 6..main as usize + 8].copy_from_slice(&table_offset.to_le_bytes());

    let image = ImageBuilder::default()
        .object(1, tads_object_data(&[], false, &[]))
        .build(main, &code, &consts);
    assert_eq!(run(&image), ("caught".to_owned(), Ok(())));
}

#[test]
fn reports_an_unhandled_vm_error() {
    let consts = Consts::default();
    let mut code = Code::default();
    let main = code.function(1, 0);
    code.op(Op::Push1).op(Op::Push0).op(Op::Div).op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    let (output, result) = run(&image);
    assert_eq!(output, "");
    assert_eq!(
        result,
        Err("Unhandled exception: division by zero".to_owned())
    );
}

#[test]
fn vm_errors_can_be_caught() {
    let mut consts = Consts::default();
    let caught = consts.string("caught: ");
    let mut code = Code::default();
    let main = code.function(1, 1);
    let try_start = code.here();
    code.op(Op::PushNil).op(Op::GetProp).u16(PROP_NAME);
    let try_end = code.here() - 1;
    code.op(Op::RetNil);
    let handler = code.here();
    code.op(Op::SetLcl1).u8(0);
    code.op(Op::PushStr).u32(caught);
    code.op(Op::GetPropLcl1)
        .u8(0)
        .u16(PROP_MESSAGE)
        .op(Op::GetR0)
        .op(Op::Add);
    code.say().op(Op::RetNil);
    let table = code.here();
    code.u16(1);
    code.u16((try_start - main as usize) as u16);
    code.u16((try_end - main as usize) as u16);
    // A handler with no class catches everything.
    code.u32(0);
    code.u16((handler - main as usize) as u16);
    let table_offset = (table - main as usize) as u16;
    code.bytes[main as usize + 6..main as usize + 8].copy_from_slice(&table_offset.to_le_bytes());

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(run(&image).0, "caught: nil object reference");
}

#[test]
fn wrong_argument_count_is_an_error() {
    let consts = Consts::default();
    let mut code = Code::default();
    let callee = code.function(2, 0);
    code.op(Op::RetNil);
    let main = code.function(1, 0);
    code.op(Op::Push1)
        .op(Op::Call)
        .u8(1)
        .u32(callee)
        .op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(
        run(&image).1,
        Err("Unhandled exception: wrong number of arguments".to_owned())
    );
}

#[test]
fn runaway_recursion_overflows_the_stack() {
    let consts = Consts::default();
    let mut code = Code::default();
    let forever = code.function(0, 0);
    code.op(Op::Call).u8(0).u32(forever).op(Op::RetNil);
    let main = code.function(1, 0);
    code.op(Op::Call).u8(0).u32(forever).op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(
        run(&image).1,
        Err("Unhandled exception: stack overflow".to_owned())
    );
}

#[test]
fn intrinsic_functions_check_their_arguments() {
    let consts = Consts::default();
    let mut code = Code::default();
    let main = code.function(1, 0);
    // dataType() with no arguments.
    code.op(Op::Builtin1)
        .u8(0)
        .u8(0)
        .u8(SET_TADS_GEN)
        .op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    assert_eq!(
        run(&image).1,
        Err("Unhandled exception: wrong number of arguments".to_owned())
    );
}

#[test]
fn unknown_metaclass_fails_to_load() {
    let mut meta = crate::meta::MetaTable::new();
    assert!(meta.add("tads-object/030005", Vec::new()).is_ok());
    let unknown = meta.add("no-such-class/030000", Vec::new());
    assert!(matches!(
        unknown,
        Err(VmError::Runtime(error)) if error.code == code::UNKNOWN_METACLASS
    ));
    let too_new = meta.add("string/999999", Vec::new());
    assert!(matches!(
        too_new,
        Err(VmError::Runtime(error)) if error.code == code::METACLASS_TOO_OLD
    ));
}

#[test]
fn identifies_itself_as_hexicon() {
    let consts = Consts::default();
    let mut code = Code::default();
    let main = code.function(1, 0);
    // tadsSay(t3GetVMID()); tadsSay(t3GetVMVsn()); tadsSay(t3GetVMBanner());
    for function in [3u8, 2, 4] {
        code.op(Op::BuiltinA).u8(0).u8(function).op(Op::GetR0).say();
        code.op(Op::PushInt8).u8(b' ').op(Op::Disc);
    }
    code.op(Op::RetNil);

    let image = ImageBuilder::default().build(main, &code, &consts);
    let version = crate::identity::version_number();
    assert_eq!(
        run(&image).0,
        format!("hexicon{version}Hexicon VM {}", crate::identity::VERSION)
    );
    // Libraries display the version by taking the number apart.
    let dotted = format!(
        "{}.{}.{}",
        version >> 16,
        (version >> 8) & 0xff,
        version & 0xff
    );
    assert_eq!(dotted, crate::identity::VERSION);
}

// ---- objects ----

#[test]
fn free_ids_are_handed_out_from_the_top_of_a_page() {
    let mut table = ObjectTable::new();
    let string = || Object::String(std::rc::Rc::from(&[][..]));
    assert!(table.alloc_with_id(ObjId(5), string(), false));
    assert!(!table.alloc_with_id(ObjId(5), string(), false));
    assert!(!table.alloc_with_id(ObjId(0), string(), false));
    assert_eq!(table.alloc(string()), ObjId(4095));
    assert_eq!(table.alloc(string()), ObjId(4094));
    // An ID past the first page adds a page, whose IDs go first.
    assert!(table.alloc_with_id(ObjId(5000), string(), false));
    assert_eq!(table.alloc(string()), ObjId(8191));
    assert!(table.is_in_root_set(ObjId(5)));
    assert!(!table.is_in_root_set(ObjId(4095)));
}

#[test]
fn properties_keep_their_order() {
    let mut object = TadsObject::new(Vec::new());
    for prop in [30u16, 10, 20, 4, 8, 15, 16, 23, 42] {
        object.set(prop, Value::Int(prop.into()));
    }
    object.set(10, Value::Nil);
    let order: Vec<u16> = object.props().iter().map(|entry| entry.prop).collect();
    assert_eq!(order, [30, 10, 20, 4, 8, 15, 16, 23, 42]);
    assert_eq!(object.get(10), Some(Value::Nil));
    assert_eq!(object.get(42), Some(Value::Int(42)));
    assert_eq!(object.get(99), None);
}

/// Build a table of program-defined objects from `(id, superclasses)`.
fn class_table(classes: &[(u32, &[u32])]) -> ObjectTable {
    let mut table = ObjectTable::new();
    for (id, superclasses) in classes {
        let mut object = TadsObject::new(superclasses.iter().map(|&s| ObjId(s)).collect());
        object.set(1, Value::Int(*id as i32));
        table.alloc_with_id(ObjId(*id), Object::Tads(object), false);
    }
    table
}

fn search_order(table: &ObjectTable, start: u32) -> Vec<u32> {
    let mut search = tads::Search::new(table, ObjId(start));
    let mut order = Vec::new();
    while search.advance() {
        order.push(search.current.0);
    }
    order
}

#[test]
fn single_inheritance_searches_up_the_chain() {
    let table = class_table(&[(1, &[]), (2, &[1]), (3, &[2])]);
    assert_eq!(search_order(&table, 3), [2, 1]);
    assert!(tads::inherits_from(&table, ObjId(3), ObjId(1)));
    assert!(!tads::inherits_from(&table, ObjId(1), ObjId(3)));
    assert!(!tads::inherits_from(&table, ObjId(3), ObjId(3)));
}

#[test]
fn a_shared_base_class_is_searched_last() {
    // D: B, C; B: A; C: A. The base class A comes after both B and C.
    let table = class_table(&[(1, &[]), (2, &[1]), (3, &[1]), (4, &[2, 3])]);
    assert_eq!(search_order(&table, 4), [2, 3, 1]);
    assert_eq!(
        tads::find_prop(&table, ObjId(4), 1),
        Some((Value::Int(4), ObjId(4)))
    );
    // What B's definition overrides, from D's point of view, is C's.
    assert_eq!(
        tads::find_inherited_prop(&table, ObjId(4), ObjId(2), 1),
        Some((Value::Int(3), ObjId(3)))
    );
    assert_eq!(
        tads::find_inherited_prop(&table, ObjId(4), ObjId(1), 1),
        None
    );
}

#[test]
fn inheritance_order_is_breadth_first_by_declaration() {
    // E: C, D; C: A; D: B.
    let table = class_table(&[(1, &[]), (2, &[]), (3, &[1]), (4, &[2]), (5, &[3, 4])]);
    assert_eq!(search_order(&table, 5), [3, 1, 4, 2]);
}

// ---- errors ----

#[test]
fn error_messages_take_parameters() {
    let message = format_message(
        "missing named argument '%s'",
        &[ErrParam::Str("x".to_owned())],
    );
    assert_eq!(message, "missing named argument 'x'");
    let message = format_message("index %d, hex %x", &[ErrParam::Int(10), ErrParam::Int(255)]);
    assert_eq!(message, "index 10, hex ff");
    // With no parameters left, the rest is copied as it stands.
    assert_eq!(format_message("100% of %s", &[]), "100% of %s");
    assert_eq!(code::message(code::NIL_DEREF), Some("nil object reference"));
    assert_eq!(code::message(1), None);
}

// ---- the console ----

fn format(texts: &[&str]) -> String {
    let mut formatter = Formatter::new(80);
    let mut output = String::new();
    for text in texts {
        formatter.format_text(text.as_bytes());
        output.push_str(&formatter.take_output());
    }
    formatter.flush(Newline::None);
    output.push_str(&formatter.take_output());
    output
}

#[test]
fn console_collapses_spaces_and_newlines() {
    assert_eq!(format(&["a   b\n\n\nc"]), "a b\nc");
    // Spaces at the start and end of a line are dropped.
    assert_eq!(format(&["  indented  \nnext"]), "indented\nnext");
    // Newlines before any text are dropped.
    assert_eq!(format(&["\n\nfirst"]), "first");
}

#[test]
fn console_writes_blank_lines_on_request() {
    assert_eq!(format(&["a\u{b}b"]), "a\n\nb");
    assert_eq!(format(&["a<br>b"]), "a\nb");
    assert_eq!(format(&["a<br><br>b"]), "a\n\nb");
    assert_eq!(format(&["a<p>b"]), "a\n\nb");
    assert_eq!(format(&["a<br height=2>b"]), "a\n\n\nb");
    assert_eq!(format(&["a<br height=0>b"]), "a\nb");
}

#[test]
fn console_keeps_quoted_spaces() {
    assert_eq!(format(&["a\u{15}\u{15}\u{15}b"]), "a   b");
    assert_eq!(format(&["a &nbsp; b"]), "a   b");
    // A tab goes to the next column that is one short of a multiple of 4.
    assert_eq!(format(&["x\ty"]), "x  y");
    assert_eq!(format(&["xyz\ty"]), "xyz    y");
}

#[test]
fn console_changes_case_on_request() {
    assert_eq!(format(&["\u{f}hello \u{e}WORLD"]), "Hello wORLD");
    // The request waits for the next letter.
    assert_eq!(format(&["\u{f}", "\"quoted\""]), "\"Quoted\"");
}

#[test]
fn console_translates_entities() {
    assert_eq!(format(&["&lt;b&gt; &amp; &quot;x&quot;"]), "<b> & \"x\"");
    assert_eq!(format(&["&#65;&#x42;&mdash;"]), "AB\u{2014}");
    // Not an entity: the ampersand is literal.
    assert_eq!(
        format(&["fish & chips &nosuch; &"]),
        "fish & chips &nosuch; &"
    );
}

#[test]
fn console_skips_tags_it_does_not_display() {
    assert_eq!(format(&["<b>bold</b> <i>it</i>"]), "bold it");
    assert_eq!(format(&["<a href=\"x > y\">link</a>"]), "link");
    assert_eq!(format(&["<title>Not shown</title>shown"]), "shown");
    assert_eq!(format(&["<img src='x.png' alt='a picture'>"]), "a picture");
    assert_eq!(
        format(&["He said <q>hi <q>there</q></q>"]),
        "He said \u{201c}hi \u{2018}there\u{2019}\u{201d}"
    );
    // A tag can be split across calls, though not within its name.
    assert_eq!(format(&["a<br ", "height=2>b"]), "a\n\n\nb");
    assert_eq!(format(&["a<b", "r>b"]), "ab");
}

#[test]
fn console_wraps_long_lines() {
    let words = "word ".repeat(20);
    let output = format(&[&words]);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].len(), 79);
    assert!(lines[0].ends_with("word"));
    assert_eq!(lines[1], "word word word word");

    // A word too long for a line is cut at the width.
    let long = "x".repeat(100);
    let output = format(&[&long]);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0].len(), 79);
    assert_eq!(lines[1].len(), 21);

    // A line can break after a hyphen.
    let hyphenated = format!("{} well-known", "a".repeat(72));
    assert_eq!(
        format(&[&hyphenated]),
        format!("{} well-\nknown", "a".repeat(72))
    );
}

#[test]
fn console_aligns_tabs() {
    assert_eq!(format(&["ab<tab id=t>cd\nx<tab to=t>y"]), "abcd\nx y");
    assert_eq!(format(&["a<tab indent=3>b"]), "a   b");
    let right = format(&["<tab align=right>end\n"]);
    assert_eq!(right, format!("{}end\n", " ".repeat(76)));
}

// ---- regular expressions ----

use crate::lookup::LookupTable;
use crate::regex::{GROUP_COUNT, Pattern};

/// Match a pattern at the start of text. Returns the matched length.
fn rex_match(pattern: &str, text: &str) -> Option<usize> {
    let mut groups = [None; GROUP_COUNT];
    Pattern::compile(pattern.as_bytes()).match_at(text.as_bytes(), 0, text.len(), true, &mut groups)
}

/// Search text for a pattern. Returns the matched text.
fn rex_search<'a>(pattern: &str, text: &'a str) -> Option<&'a str> {
    let mut groups = [None; GROUP_COUNT];
    Pattern::compile(pattern.as_bytes())
        .search(text.as_bytes(), 0, true, &mut groups)
        .map(|(start, len)| &text[start..start + len])
}

#[test]
fn regex_matches_literals_and_closures() {
    assert_eq!(rex_match("abc", "abcdef"), Some(3));
    assert_eq!(rex_match("abc", "abd"), None);
    assert_eq!(rex_match("a*b", "aaab"), Some(4));
    assert_eq!(rex_match("a*", "bbb"), Some(0));
    assert_eq!(rex_match("a+", "bbb"), None);
    assert_eq!(rex_match("ab?c", "ac"), Some(2));
    assert_eq!(rex_match("x.z", "xyz"), Some(3));
    // The longest alternative wins, not the first.
    assert_eq!(rex_match("a|ab|abc", "abcd"), Some(3));
    // An empty pattern matches nothing, successfully.
    assert_eq!(rex_match("", "abc"), Some(0));
}

#[test]
fn regex_understands_classes_and_ranges() {
    assert_eq!(rex_match("<alpha>+", "hello world"), Some(5));
    assert_eq!(rex_match("%d+", "12345x"), Some(5));
    assert_eq!(rex_match("<alpha|digit>+", "abc123!"), Some(6));
    assert_eq!(rex_match("<^space>+", "two words"), Some(3));
    assert_eq!(rex_match("[a-c]+", "abcabd"), Some(5));
    assert_eq!(rex_match("[^a-c]+", "xyzabc"), Some(3));
    assert_eq!(rex_match("<lparen>x<rparen>", "(x)"), Some(3));
    assert_eq!(rex_match("%w+", "caf\u{e9} au lait"), Some(5));
    assert_eq!(rex_match("%.", "."), Some(1));
    assert_eq!(rex_match("%.", "x"), None);
}

#[test]
fn regex_honors_flags_and_intervals() {
    assert_eq!(rex_match("<nocase>hello", "HeLLo"), Some(5));
    assert_eq!(rex_match("hello", "HeLLo"), None);
    assert_eq!(rex_match("a{2,3}", "aaaa"), Some(3));
    assert_eq!(rex_match("a{2}", "a"), None);
    assert_eq!(rex_match("a{2,}", "aaaaa"), Some(5));
    // A shortest-match closure gives way to what follows it.
    assert_eq!(rex_match("a.*?c", "abcabc"), Some(3));
    assert_eq!(rex_match("a.*c", "abcabc"), Some(6));
    assert_eq!(rex_match("<min>a.*c", "abcabc"), Some(3));
}

#[test]
fn regex_records_groups() {
    let text = "key=value";
    let mut groups = [None; GROUP_COUNT];
    let pattern = Pattern::compile(b"(<alpha>+)=(<alpha>+)");
    assert_eq!(pattern.group_count, 2);
    assert_eq!(
        pattern.match_at(text.as_bytes(), 0, text.len(), true, &mut groups),
        Some(9)
    );
    assert_eq!(groups[0], Some((0, 3)));
    assert_eq!(groups[1], Some((4, 9)));
    // A group can be matched again later in the pattern.
    assert_eq!(rex_match("(a+)-%1", "aa-aa"), Some(5));
    assert_eq!(rex_match("(a+)-%1", "aa-a"), None);
    // A group that is not captured.
    assert_eq!(Pattern::compile(b"(?:ab)+(c)").group_count, 1);
}

#[test]
fn regex_anchors_and_assertions() {
    assert_eq!(rex_search("^abc", "abcabc"), Some("abc"));
    assert_eq!(rex_search("abc$", "abcab"), None);
    assert_eq!(rex_search("%<the%>", "other the end"), Some("the"));
    assert_eq!(rex_search("a(?=b)", "ac ab"), Some("a"));
    assert_eq!(rex_search("a(?!b).", "ab ac"), Some("ac"));
    assert_eq!(rex_search("(?<=x)a", "ya xa"), Some("a"));
    // A dollar sign in the middle is just a dollar sign.
    assert_eq!(rex_search("a$b", "xa$b"), Some("a$b"));
}

#[test]
fn regex_searches_forward_and_backward() {
    assert_eq!(rex_search("%d+", "abc 123 def 4567"), Some("123"));
    assert_eq!(rex_search("z", "abc"), None);
    let text = "abc 123 def 4567 ghi";
    let mut groups = [None; GROUP_COUNT];
    let pattern = Pattern::compile(b"%d+");
    assert_eq!(
        pattern.search_back(text.as_bytes(), text.len(), true, &mut groups),
        Some((12, 4))
    );
    // A backward search cannot use text past its starting point.
    assert_eq!(
        pattern.search_back(text.as_bytes(), 14, true, &mut groups),
        Some((12, 2))
    );
}

#[test]
fn regex_survives_empty_loops() {
    // Patterns that could loop forever without consuming anything.
    assert_eq!(rex_match("(a*)*b", "aab"), Some(3));
    assert_eq!(rex_match("(a*)+", "b"), Some(0));
    assert_eq!(rex_match("(|a)*", "aaa"), Some(3));
}

// ---- lookup tables ----

#[test]
fn new_lookup_table_is_empty() {
    let table = LookupTable::new(8, 4, false);
    assert_eq!(table.bucket_count(), 8);
    assert_eq!(table.slot_count(), 4);
    assert!(table.entries().is_empty());
    assert_eq!(table.slot(0), None);
}

// ---- random numbers ----

#[test]
fn random_numbers_repeat_until_seeded() {
    let mut a = crate::rng::Isaac::new();
    let mut b = crate::rng::Isaac::new();
    let first: Vec<u32> = (0..600).map(|_| a.next()).collect();
    let second: Vec<u32> = (0..600).map(|_| b.next()).collect();
    assert_eq!(first, second);
    // The values are not all the same, and a seed changes them.
    assert!(first.iter().any(|&value| value != first[0]));
    let mut seeded = crate::rng::Isaac::seeded(&[1, 2, 3, 4]);
    let third: Vec<u32> = (0..600).map(|_| seeded.next()).collect();
    assert_ne!(first, third);
}

// ---- BigNumber ----

use crate::bignum::Big;
use crate::bignum_math::{self, Cache};

fn big(text: &str) -> Big {
    Big::parse(text, Big::precision_from_str(text))
}

#[test]
fn bignumber_arithmetic_keeps_the_larger_precision() {
    assert_eq!(Big::add(&big("1.5"), &big("2.25")).to_text(), "3.75");
    assert_eq!(Big::sub(&big("1.5"), &big("2.25")).to_text(), "-.75");
    assert_eq!(Big::mul(&big("1.5"), &big("2.25")).to_text(), "3.38");
    let third = Big::div(&big("1.0000"), &big("3")).expect("not dividing by zero");
    assert_eq!(third.to_text(), ".33333");
    assert!(Big::div(&big("1"), &big("0")).is_none());
}

#[test]
fn bignumber_holds_what_an_integer_cannot() {
    let limit = Big::from_int(i32::MAX.into(), 10);
    let one = Big::from_int(1, 10);
    let past = Big::add(&limit, &one);
    assert_eq!(past.to_text(), "2147483648");
    assert!(past.to_i32().1, "conversion to an integer overflows");
    assert_eq!(limit.to_i32(), (i32::MAX, false));
}

#[test]
fn bignumber_reads_other_radixes() {
    assert_eq!(Big::parse_radix("ff", 16).to_text(), "255");
    assert_eq!(
        Big::parse_radix("ffffffffffffffff", 16).to_text(),
        "18446744073709551615"
    );
    assert_eq!(Big::parse_radix(" -101", 2).to_text(), "-5");
    assert_eq!(Big::parse_radix("ff", 16).to_radix_text(2), "11111111");
}

#[test]
fn bignumber_computes_constants_and_roots() {
    let mut cache = Cache::default();
    let pi = cache.pi(10).expect("pi").with_prec(10, true);
    assert_eq!(pi.to_text(), "3.141592654");
    let e = cache.e(10).expect("e").with_prec(10, true);
    assert_eq!(e.to_text(), "2.718281828");
    let root = bignum_math::sqrt_into(10, &big("2.000000000")).expect("root");
    assert_eq!(root.to_text(), "1.414213562");
    assert!(bignum_math::sqrt_into(10, &big("-2")).is_err());
}

#[test]
fn case_folding_expands_characters() {
    assert_eq!(crate::casefold::fold('A'), Some("a"));
    assert_eq!(crate::casefold::fold('ß'), Some("ss"));
    assert_eq!(crate::casefold::fold('a'), None);
}

// ---- hashes and byte packing ----

#[test]
fn hashes_match_the_standard_answers() {
    use crate::hash::{hex, md5, sha256};
    assert_eq!(hex(&md5(b"")), "D41D8CD98F00B204E9800998ECF8427E");
    assert_eq!(hex(&md5(b"abc")), "900150983CD24FB0D6963F7D28E17F72");
    assert_eq!(
        hex(&sha256(b"abc")),
        "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
    );
    // A message long enough to need a second block.
    let long = [b'a'; 100];
    assert_eq!(hex(&md5(&long)), "36A92CC94A9E0FA21F625F8BFB007ADF");
    assert_eq!(
        hex(&sha256(&long)),
        "2816597888E4A0D3A36B82B83316AB32680EB8F00F8CD3B904D681246D285A0E"
    );
}

#[test]
fn bignumber_converts_to_and_from_binary() {
    let mut cache = Cache::default();
    let (bytes, overflow) = bignum_math::to_ieee754(&big("1.5"), 64, &mut cache).expect("double");
    assert!(!overflow);
    assert_eq!(bytes, 1.5f64.to_le_bytes());
    let (bytes, _) = bignum_math::to_ieee754(&big("-0.15625"), 32, &mut cache).expect("float");
    assert_eq!(bytes, (-0.15625f32).to_le_bytes());
    let back = bignum_math::from_ieee754(&2.5f64.to_le_bytes(), &mut cache).expect("double");
    assert_eq!(back.to_text(), "2.5");

    let (bytes, overflow) = big("-2").to_p8(true);
    assert_eq!((bytes, overflow), ((-2i64).to_le_bytes(), false));
    assert_eq!(
        Big::from_p8(u64::MAX.to_le_bytes(), false).to_text(),
        "18446744073709551615"
    );
    assert_eq!(Big::from_p8(u64::MAX.to_le_bytes(), true).to_text(), "-1");

    let (ber, overflow) = big("300").to_ber();
    assert_eq!((ber, overflow), (vec![0x82, 0x2c], false));
    assert_eq!(Big::from_ber(&[2, 0x2c]).to_text(), "300");
}

// ---- character sets ----

use crate::charset::CharMap;

#[test]
fn character_sets_go_by_many_names() {
    for name in ["latin-2", "ISO-8859-2", "iso2", "iso_8859_2"] {
        assert!(CharMap::load(name).is_some(), "{name}");
    }
    for name in ["cp1252", "windows-1252", "win1252", "1252"] {
        assert!(CharMap::load(name).is_some(), "{name}");
    }
    for name in ["utf-8", "UTF8", "utf-16le", "unicodeb", "ascii", "us", "l1"] {
        assert!(CharMap::load(name).is_some(), "{name}");
    }
    // `l2` looks like `l1`, but only that one is a name the reference knows.
    for name in ["", "nonesuch", "utf-16", "latin-0", "cp99999", "l2"] {
        assert!(CharMap::load(name).is_none(), "{name}");
    }
}

#[test]
fn character_sets_convert_text_and_bytes() {
    let latin2 = CharMap::load("latin-2").expect("latin-2");
    assert_eq!(latin2.encode("A\u{15e}\u{164}"), [0x41, 0xaa, 0xab]);
    assert_eq!(latin2.decode(&[0x41, 0xaa, 0xab]), "A\u{15e}\u{164}");

    // A set stands in for some characters it lacks, and writes a
    // question mark for the rest.
    let ascii = CharMap::load("us-ascii").expect("us-ascii");
    assert_eq!(ascii.encode("d\u{e9}j\u{e0} \u{4e2d}"), b"deja ?");
    assert_eq!(ascii.decode(&[0x41, 0xe9]), "A\u{fffd}");

    let utf16 = CharMap::load("utf-16be").expect("utf-16be");
    assert_eq!(utf16.encode("A\u{20ac}"), [0x00, 0x41, 0x20, 0xac]);
    assert_eq!(utf16.decode(&[0x00, 0x41, 0x20, 0xac, 0x00]), "A\u{20ac}");

    // UTF-8 that is cut short loses the unfinished character, and
    // anything else that is not UTF-8 becomes a question mark.
    let utf8 = CharMap::load("utf-8").expect("utf-8");
    assert_eq!(utf8.decode(&[0x41, 0xc3, 0xa9, 0xc3]), "A\u{e9}");
    assert_eq!(utf8.decode(&[0x41, 0xff, 0x42]), "A?B");
}

// ---- undo and garbage collection ----

fn vector_of(table: &ObjectTable, id: ObjId) -> Vec<Value> {
    match table.get(id) {
        Some(Object::Vector(items)) => items.clone(),
        other => panic!("not a vector: {other:?}"),
    }
}

fn push_to(table: &mut ObjectTable, id: ObjId, value: Value) {
    match table.get_mut(id) {
        Some(Object::Vector(items)) => items.push(value),
        other => panic!("not a vector: {other:?}"),
    }
}

#[test]
fn undo_returns_objects_to_the_savepoint() {
    let mut table = ObjectTable::new();
    let old = table.alloc(Object::Vector(vec![Value::Int(1)]));
    assert!(!table.undo(), "nothing to undo without a savepoint");

    table.savepoint();
    push_to(&mut table, old, Value::Int(2));
    table.savepoint();
    push_to(&mut table, old, Value::Int(3));
    push_to(&mut table, old, Value::Int(4));
    // An object made after a savepoint is not part of it.
    let new = table.alloc(Object::Vector(Vec::new()));
    push_to(&mut table, new, Value::Int(9));

    assert!(table.undo());
    assert_eq!(vector_of(&table, old), [Value::Int(1), Value::Int(2)]);
    assert_eq!(vector_of(&table, new), [Value::Int(9)]);
    assert!(table.undo());
    assert_eq!(vector_of(&table, old), [Value::Int(1)]);
    assert!(!table.undo());
}

#[test]
fn collector_frees_what_nothing_refers_to() {
    let mut table = ObjectTable::new();
    let kept = table.alloc(Object::Vector(Vec::new()));
    let inner = table.alloc(Object::String(b"inner".as_slice().into()));
    push_to(&mut table, kept, Value::Obj(inner));
    let garbage_a = table.alloc(Object::String(b"a".as_slice().into()));
    let garbage_b = table.alloc(Object::String(b"b".as_slice().into()));

    let candidates = table.collect_mark(&[Value::Obj(kept)], false);
    assert_eq!(candidates, [garbage_b, garbage_a]);
    let finalizers = table.collect_sweep(&candidates, &[]);
    assert!(finalizers.is_empty());
    assert!(table.get(kept).is_some() && table.get(inner).is_some());
    assert!(table.get(garbage_a).is_none() && table.get(garbage_b).is_none());

    // Freed IDs are handed out again, the highest first.
    assert_eq!(table.alloc(Object::Vector(Vec::new())), garbage_a);
    assert_eq!(table.alloc(Object::Vector(Vec::new())), garbage_b);
}

#[test]
fn collector_keeps_an_object_until_its_finalizer_has_run() {
    let mut table = ObjectTable::new();
    let dying = table.alloc(Object::Vector(Vec::new()));
    let held = table.alloc(Object::String(b"held".as_slice().into()));
    push_to(&mut table, dying, Value::Obj(held));

    // Unreachable, with a finalizer: it and what it refers to stay for
    // the finalizer to use.
    let candidates = table.collect_mark(&[], false);
    let queue = table.collect_sweep(&candidates, &[dying]);
    assert_eq!(queue, [dying]);
    assert!(table.get(dying).is_some() && table.get(held).is_some());

    // The next collection takes both.
    let candidates = table.collect_mark(&[], false);
    assert!(candidates.is_empty(), "nothing is finalized twice");
    assert!(table.collect_sweep(&candidates, &[]).is_empty());
    assert!(table.get(dying).is_none() && table.get(held).is_none());
}

#[test]
fn collector_spares_what_a_savepoint_can_bring_back() {
    let mut table = ObjectTable::new();
    let holder = table.alloc(Object::Vector(Vec::new()));
    let item = table.alloc(Object::String(b"item".as_slice().into()));
    push_to(&mut table, holder, Value::Obj(item));
    table.savepoint();
    match table.get_mut(holder) {
        Some(Object::Vector(items)) => items.clear(),
        _ => unreachable!(),
    }

    let candidates = table.collect_mark(&[Value::Obj(holder)], false);
    table.collect_sweep(&candidates, &[]);
    assert!(table.get(item).is_some());
    assert!(table.undo());
    assert_eq!(vector_of(&table, holder), [Value::Obj(item)]);
}

#[test]
fn collector_empties_weak_tables_of_dead_values() {
    let mut table = ObjectTable::new();
    let mut weak = LookupTable::new(8, 8, true);
    let value = table.alloc(Object::String(b"value".as_slice().into()));
    weak.insert_for_test(Value::Int(1), Value::Obj(value));
    weak.insert_for_test(Value::Int(2), Value::Int(7));
    let weak = table.alloc(Object::LookupTable(Box::new(weak)));

    let candidates = table.collect_mark(&[Value::Obj(weak)], false);
    table.collect_sweep(&candidates, &[]);
    assert!(table.get(value).is_none());
    match table.get(weak) {
        Some(Object::LookupTable(weak)) => {
            assert_eq!(weak.entries(), [(Value::Int(2), Value::Int(7))]);
        }
        _ => unreachable!(),
    }
}
