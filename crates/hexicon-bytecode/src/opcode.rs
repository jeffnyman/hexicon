//! The T3 VM instruction set.
//!
//! The opcode numbers and operand layouts follow the T3 VM "Byte-Code
//! Instruction Set" document and the reference interpreter's `vmop.h` and
//! `vmrun.cpp`. A few instructions are in the interpreter but not in the
//! document (`iternext`, `getsetlcl1`, `getsetlcl1r0`, `dupr0`, and
//! `getlcln0` through `getlcln5`); their layouts are taken from the
//! interpreter.

/// The kind and size of one fixed operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandKind {
    /// Unsigned 8-bit number.
    U8,
    /// Signed 8-bit number.
    S8,
    /// Unsigned 16-bit number.
    U16,
    /// Signed 32-bit number.
    S32,
    /// Argument count, 8 bits.
    Argc8,
    /// Argument count, 16 bits.
    Argc16,
    /// Local variable number, 8 bits.
    Local8,
    /// Local variable number, 16 bits.
    Local16,
    /// Parameter number, 8 bits.
    Arg8,
    /// Parameter number, 16 bits.
    Arg16,
    /// Constant pool offset of a string, 32 bits.
    String,
    /// Constant pool offset of a list, 32 bits.
    List,
    /// Object ID, 32 bits.
    Object,
    /// Property ID, 16 bits.
    Property,
    /// Code pool offset of a function, 32 bits.
    Function,
    /// Enumerator ID, 32 bits.
    Enum,
    /// Signed 16-bit branch offset, relative to the operand's own address.
    Branch,
    /// Unsigned 16-bit offset to a named argument table, relative to the
    /// operand's own address.
    TableOffset,
    /// Index into the image's metaclass dependency list, 8 bits.
    Metaclass8,
    /// Index into the image's metaclass dependency list, 16 bits.
    Metaclass16,
    /// Index into the image's function set dependency list, 8 bits.
    FunctionSet8,
    /// Index into the image's function set dependency list, 16 bits.
    FunctionSet16,
    /// Index of a function within its function set, 8 bits.
    FunctionIndex8,
    /// Index of a function within its function set, 16 bits.
    FunctionIndex16,
}

impl OperandKind {
    /// Encoded size in bytes.
    pub const fn size(self) -> usize {
        match self {
            Self::U8
            | Self::S8
            | Self::Argc8
            | Self::Local8
            | Self::Arg8
            | Self::Metaclass8
            | Self::FunctionSet8
            | Self::FunctionIndex8 => 1,
            Self::U16
            | Self::Argc16
            | Self::Local16
            | Self::Arg16
            | Self::Property
            | Self::Branch
            | Self::TableOffset
            | Self::Metaclass16
            | Self::FunctionSet16
            | Self::FunctionIndex16 => 2,
            Self::S32 | Self::String | Self::List | Self::Object | Self::Function | Self::Enum => 4,
        }
    }
}

/// How an instruction's operands are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// A fixed sequence of operands.
    Fixed(&'static [OperandKind]),
    /// `pushstri`: a 16-bit length followed by that many bytes of text.
    InlineString,
    /// `switch`: a 16-bit case count, that many cases of a five-byte value
    /// and a branch offset, then a default branch offset.
    Switch,
    /// `namedargtab`: a 16-bit byte count followed by a table of argument names.
    NamedArgTable,
}

macro_rules! opcodes {
    ($( $code:literal $variant:ident $name:literal $layout:tt ; )*) => {
        /// A T3 VM instruction.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        #[allow(missing_docs)]
        pub enum Opcode {
            $( $variant = $code, )*
        }

        impl Opcode {
            /// Every defined opcode, in numeric order.
            pub const ALL: &'static [Opcode] = &[ $( Opcode::$variant, )* ];

            /// The opcode for a byte, if the byte is a defined instruction.
            pub const fn from_byte(byte: u8) -> Option<Self> {
                match byte {
                    $( $code => Some(Self::$variant), )*
                    _ => None,
                }
            }

            /// The instruction's mnemonic.
            pub const fn name(self) -> &'static str {
                match self {
                    $( Self::$variant => $name, )*
                }
            }

            /// How the instruction's operands are laid out.
            pub const fn layout(self) -> Layout {
                match self {
                    $( Self::$variant => opcodes!(@layout $layout), )*
                }
            }
        }
    };
    (@layout [ $( $kind:ident ),* ]) => { Layout::Fixed(&[ $( OperandKind::$kind ),* ]) };
    (@layout InlineString) => { Layout::InlineString };
    (@layout Switch) => { Layout::Switch };
    (@layout NamedArgTable) => { Layout::NamedArgTable };
}

opcodes! {
    0x01 Push0 "push_0" [];
    0x02 Push1 "push_1" [];
    0x03 PushInt8 "pushint8" [S8];
    0x04 PushInt "pushint" [S32];
    0x05 PushStr "pushstr" [String];
    0x06 PushLst "pushlst" [List];
    0x07 PushObj "pushobj" [Object];
    0x08 PushNil "pushnil" [];
    0x09 PushTrue "pushtrue" [];
    0x0A PushPropId "pushpropid" [Property];
    0x0B PushFnPtr "pushfnptr" [Function];
    0x0C PushStrI "pushstri" InlineString;
    0x0D PushParLst "pushparlst" [U8];
    0x0E MakeLstPar "makelstpar" [];
    0x0F PushEnum "pushenum" [Enum];
    0x10 PushBifPtr "pushbifptr" [FunctionIndex16, FunctionSet16];

    0x20 Neg "neg" [];
    0x21 BNot "bnot" [];
    0x22 Add "add" [];
    0x23 Sub "sub" [];
    0x24 Mul "mul" [];
    0x25 BAnd "band" [];
    0x26 BOr "bor" [];
    0x27 Shl "shl" [];
    0x28 AShr "ashr" [];
    0x29 Xor "xor" [];
    0x2A Div "div" [];
    0x2B Mod "mod" [];
    0x2C Not "not" [];
    0x2D Boolize "boolize" [];
    0x2E Inc "inc" [];
    0x2F Dec "dec" [];
    0x30 LShr "lshr" [];

    0x40 Eq "eq" [];
    0x41 Ne "ne" [];
    0x42 Lt "lt" [];
    0x43 Le "le" [];
    0x44 Gt "gt" [];
    0x45 Ge "ge" [];

    0x50 RetVal "retval" [];
    0x51 RetNil "retnil" [];
    0x52 RetTrue "rettrue" [];
    0x54 Ret "ret" [];
    0x56 NamedArgPtr "namedargptr" [Argc8, TableOffset];
    0x57 NamedArgTab "namedargtab" NamedArgTable;
    0x58 Call "call" [Argc8, Function];
    0x59 PtrCall "ptrcall" [Argc8];

    0x60 GetProp "getprop" [Property];
    0x61 CallProp "callprop" [Argc8, Property];
    0x62 PtrCallProp "ptrcallprop" [Argc8];
    0x63 GetPropSelf "getpropself" [Property];
    0x64 CallPropSelf "callpropself" [Argc8, Property];
    0x65 PtrCallPropSelf "ptrcallpropself" [Argc8];
    0x66 ObjGetProp "objgetprop" [Object, Property];
    0x67 ObjCallProp "objcallprop" [Argc8, Object, Property];
    0x68 GetPropData "getpropdata" [Property];
    0x69 PtrGetPropData "ptrgetpropdata" [];
    0x6A GetPropLcl1 "getproplcl1" [Local8, Property];
    0x6B CallPropLcl1 "callproplcl1" [Argc8, Local8, Property];
    0x6C GetPropR0 "getpropr0" [Property];
    0x6D CallPropR0 "callpropr0" [Argc8, Property];

    0x72 Inherit "inherit" [Argc8, Property];
    0x73 PtrInherit "ptrinherit" [Argc8];
    0x74 ExpInherit "expinherit" [Argc8, Property, Object];
    0x75 PtrExpInherit "ptrexpinherit" [Argc8, Object];
    0x76 VarArgc "varargc" [];
    0x77 Delegate "delegate" [Argc8, Property];
    0x78 PtrDelegate "ptrdelegate" [Argc8];

    0x7A Swap2 "swap2" [];
    0x7B SwapN "swapn" [U8, U8];
    0x7C GetArgN0 "getargn0" [];
    0x7D GetArgN1 "getargn1" [];
    0x7E GetArgN2 "getargn2" [];
    0x7F GetArgN3 "getargn3" [];

    0x80 GetLcl1 "getlcl1" [Local8];
    0x81 GetLcl2 "getlcl2" [Local16];
    0x82 GetArg1 "getarg1" [Arg8];
    0x83 GetArg2 "getarg2" [Arg16];
    0x84 PushSelf "pushself" [];
    0x85 GetDbLcl "getdblcl" [Local16, U16];
    0x86 GetDbArg "getdbarg" [Arg16, U16];
    0x87 GetArgc "getargc" [];
    0x88 Dup "dup" [];
    0x89 Disc "disc" [];
    0x8A Disc1 "disc1" [U8];
    0x8B GetR0 "getr0" [];
    0x8C GetDbArgc "getdbargc" [U16];
    0x8D Swap "swap" [];
    0x8E PushCtxEle "pushctxele" [U8];
    0x8F Dup2 "dup2" [];

    0x90 Switch "switch" Switch;
    0x91 Jmp "jmp" [Branch];
    0x92 Jt "jt" [Branch];
    0x93 Jf "jf" [Branch];
    0x94 Je "je" [Branch];
    0x95 Jne "jne" [Branch];
    0x96 Jgt "jgt" [Branch];
    0x97 Jge "jge" [Branch];
    0x98 Jlt "jlt" [Branch];
    0x99 Jle "jle" [Branch];
    0x9A Jst "jst" [Branch];
    0x9B Jsf "jsf" [Branch];
    0x9C LJsr "ljsr" [Branch];
    0x9D LRet "lret" [Local16];
    0x9E JNil "jnil" [Branch];
    0x9F JNotNil "jnotnil" [Branch];
    0xA0 JR0T "jr0t" [Branch];
    0xA1 JR0F "jr0f" [Branch];
    0xA2 IterNext "iternext" [Local16, Branch];
    0xA3 GetSetLcl1R0 "getsetlcl1r0" [Local8];
    0xA4 GetSetLcl1 "getsetlcl1" [Local8];
    0xA5 DupR0 "dupr0" [];
    0xA6 GetSpN "getspn" [U8];

    0xAA GetLclN0 "getlcln0" [];
    0xAB GetLclN1 "getlcln1" [];
    0xAC GetLclN2 "getlcln2" [];
    0xAD GetLclN3 "getlcln3" [];
    0xAE GetLclN4 "getlcln4" [];
    0xAF GetLclN5 "getlcln5" [];

    0xB0 Say "say" [String];
    0xB1 BuiltinA "builtin_a" [Argc8, FunctionIndex8];
    0xB2 BuiltinB "builtin_b" [Argc8, FunctionIndex8];
    0xB3 BuiltinC "builtin_c" [Argc8, FunctionIndex8];
    0xB4 BuiltinD "builtin_d" [Argc8, FunctionIndex8];
    0xB5 Builtin1 "builtin1" [Argc8, FunctionIndex8, FunctionSet8];
    0xB6 Builtin2 "builtin2" [Argc8, FunctionIndex16, FunctionSet8];
    0xB7 CallExt "callext" [];
    0xB8 Throw "throw" [];
    0xB9 SayVal "sayval" [];
    0xBA Index "index" [];
    0xBB IdxLcl1Int8 "idxlcl1int8" [Local8, U8];
    0xBC IdxInt8 "idxint8" [U8];

    0xC0 New1 "new1" [Argc8, Metaclass8];
    0xC1 New2 "new2" [Argc16, Metaclass16];
    0xC2 TrNew1 "trnew1" [Argc8, Metaclass8];
    0xC3 TrNew2 "trnew2" [Argc16, Metaclass16];

    0xD0 IncLcl "inclcl" [Local16];
    0xD1 DecLcl "declcl" [Local16];
    0xD2 AddILcl1 "addilcl1" [Local8, S8];
    0xD3 AddILcl4 "addilcl4" [Local16, S32];
    0xD4 AddToLcl "addtolcl" [Local16];
    0xD5 SubFromLcl "subfromlcl" [Local16];
    0xD6 ZeroLcl1 "zerolcl1" [Local8];
    0xD7 ZeroLcl2 "zerolcl2" [Local16];
    0xD8 NilLcl1 "nillcl1" [Local8];
    0xD9 NilLcl2 "nillcl2" [Local16];
    0xDA OneLcl1 "onelcl1" [Local8];
    0xDB OneLcl2 "onelcl2" [Local16];

    0xE0 SetLcl1 "setlcl1" [Local8];
    0xE1 SetLcl2 "setlcl2" [Local16];
    0xE2 SetArg1 "setarg1" [Arg8];
    0xE3 SetArg2 "setarg2" [Arg16];
    0xE4 SetInd "setind" [];
    0xE5 SetProp "setprop" [Property];
    0xE6 PtrSetProp "ptrsetprop" [];
    0xE7 SetPropSelf "setpropself" [Property];
    0xE8 ObjSetProp "objsetprop" [Object, Property];
    0xE9 SetDbLcl "setdblcl" [Local16, U16];
    0xEA SetDbArg "setdbarg" [Arg16, U16];
    0xEB SetSelf "setself" [];
    0xEC LoadCtx "loadctx" [];
    0xED StoreCtx "storectx" [];
    0xEE SetLcl1R0 "setlcl1r0" [Local8];
    0xEF SetIndLcl1I8 "setindlcl1i8" [Local8, U8];

    0xF1 Bp "bp" [];
    0xF2 Nop "nop" [];
}
