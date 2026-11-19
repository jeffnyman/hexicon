//! Run-time errors.
//!
//! The error numbers and message texts are the reference VM's
//! (`vmerrnum.h` and `vmerrmsg.cpp`). Programs see both: the number as the
//! `errno_` of a `RuntimeError`, and the text as its `exceptionMessage`.

use std::fmt;

use crate::value::ObjId;

/// A value substituted into an error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrParam {
    /// A number, for `%d`, `%u`, and `%x`.
    Int(i32),
    /// Text, for `%s`.
    Str(String),
}

/// An error raised by the VM itself, as opposed to an exception object a
/// program throws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    /// The error number; one of the constants in [`code`].
    pub code: u32,
    /// Values for the message's `%` fields, in order.
    pub params: Vec<ErrParam>,
}

impl RuntimeError {
    /// The error's message with its parameters filled in.
    pub fn message(&self) -> String {
        match code::message(self.code) {
            Some(template) => format_message(template, &self.params),
            None => format!("VM Error: code {}", self.code),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// Why execution stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmError {
    /// The VM raised an error. Inside a running program this becomes a
    /// `RuntimeError` exception the program can catch.
    Runtime(RuntimeError),
    /// An exception object is propagating out of byte code.
    Thrown(ObjId),
    /// The program used a part of the VM that Hexicon does not have yet.
    /// Programs cannot catch this.
    Unimplemented(String),
    /// The host has no input yet, so the program has to stop where it is
    /// until it does. This is how the instruction that asked finds its
    /// way out of the interpreter; it never reaches a program or a host.
    Suspend,
}

impl VmError {
    /// A VM error with no message parameters.
    pub fn new(code: u32) -> Self {
        Self::Runtime(RuntimeError {
            code,
            params: Vec::new(),
        })
    }

    /// A VM error with message parameters.
    pub fn with(code: u32, params: Vec<ErrParam>) -> Self {
        Self::Runtime(RuntimeError { code, params })
    }

    /// A report that a feature is missing.
    pub fn unimplemented(what: impl Into<String>) -> Self {
        Self::Unimplemented(what.into())
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => write!(f, "VM Error: {error}"),
            Self::Thrown(_) => f.write_str("unhandled exception"),
            Self::Unimplemented(what) => write!(f, "not implemented: {what}"),
            Self::Suspend => f.write_str("the program is waiting for input"),
        }
    }
}

impl std::error::Error for VmError {}

/// The result of an operation that can raise a VM error.
pub type VmResult<T> = Result<T, VmError>;

/// Fill in a message template the way the reference VM does: each `%s`,
/// `%d`, `%u`, or `%x` takes the next parameter; `%%` is a literal percent
/// sign that also consumes a parameter; and once the parameters run out,
/// the rest of the template is copied as it stands.
pub fn format_message(template: &str, params: &[ErrParam]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' || next >= params.len() {
            out.push(c);
            continue;
        }
        let param = &params[next];
        next += 1;
        let Some(spec) = chars.next() else { break };
        match (spec, param) {
            ('s', ErrParam::Str(text)) => out.push_str(text),
            ('d', ErrParam::Int(n)) => out.push_str(&n.to_string()),
            ('u', ErrParam::Int(n)) => out.push_str(&(*n as u32).to_string()),
            ('x', ErrParam::Int(n)) => out.push_str(&format!("{:x}", *n as u32)),
            ('s' | 'd' | 'u' | 'x', _) => out.push(spec),
            ('%', _) => out.push('%'),
            (other, _) => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

macro_rules! error_codes {
    ($( $name:ident = $number:literal, $message:literal; )*) => {
        /// Error numbers and their messages.
        pub mod code {
            #![allow(missing_docs)]
            $( pub const $name: u32 = $number; )*

            /// The message template for an error number.
            pub fn message(code: u32) -> Option<&'static str> {
                match code {
                    $( $number => Some($message), )*
                    _ => None,
                }
            }
        }
    };
}

error_codes! {
    READ_FILE = 101, "error reading file";
    WRITE_FILE = 102, "error writing file";
    FILE_NOT_FOUND = 103, "file not found";
    CREATE_FILE = 104, "error creating file";
    CLOSE_FILE = 105, "error closing file";
    DELETE_FILE = 106, "error deleting file";
    PACK_PARSE = 107, "data packer format string parsing error at character index %d";
    PACK_ARG_MISMATCH = 108, "data packer argument type mismatch at format string index %d";
    PACK_ARGC_MISMATCH = 109, "wrong number of data packer arguments at format string index %d";
    NET_FILE_NOIMPL = 110, "this file operation isn't supported for storage server files";
    RENAME_FILE = 111, "error renaming file";
    OBJ_IN_USE = 201, "object ID in use - the image/save file might be corrupted";
    OUT_OF_MEMORY = 202, "out of memory";
    NO_MEM_FOR_PAGE = 203, "out of memory allocating pool page";
    BAD_POOL_PAGE_SIZE = 204, "invalid page size - file is not valid";
    OUT_OF_PROPIDS = 205, "no more property ID's are available";
    CIRCULAR_INIT = 206, "circular initialization dependency in intrinsic class (internal error)";
    UNKNOWN_METACLASS = 301, "this interpreter version cannot run this program (program requires intrinsic class %s, which is not available in this interpreter)";
    UNKNOWN_FUNC_SET = 302, "this interpreter version cannot run this program (program requires intrinsic function set %s, which is not available in this interpreter)";
    READ_PAST_IMG_END = 303, "reading past end of image file - program might be corrupted";
    NOT_AN_IMAGE_FILE = 304, "this is not an image file (no valid signature found)";
    UNKNOWN_IMAGE_BLOCK = 305, "this interpreter version cannot run this program (unknown block type in image file)";
    IMAGE_BLOCK_TOO_SMALL = 306, "data block too small";
    IMAGE_POOL_BEFORE_DEF = 307, "invalid image file: pool page before pool definition";
    IMAGE_POOL_BAD_PAGE = 308, "invalid image file: pool page out of range of definition";
    IMAGE_BAD_POOL_ID = 309, "invalid image file: invalid pool ID";
    LOAD_BAD_PAGE_IDX = 310, "invalid image file: bad page index";
    LOAD_UNDEF_PAGE = 311, "loading undefined pool page";
    IMAGE_POOL_REDEF = 312, "invalid image file: pool is defined more than once";
    IMAGE_METADEP_REDEF = 313, "invalid image file: multiple intrinsic class dependency tables found";
    IMAGE_NO_METADEP = 314, "invalid image file: no intrinsic class dependency table found";
    IMAGE_FUNCDEP_REDEF = 315, "invalid image file: multiple function set dependency tables found";
    IMAGE_NO_FUNCDEP = 316, "invalid image file: no function set dependency table found";
    IMAGE_ENTRYPT_REDEF = 317, "invalid image file: multiple entrypoints found";
    IMAGE_NO_ENTRYPT = 318, "invalid image file: no entrypoint found";
    IMAGE_INCOMPAT_VSN = 319, "incompatible image file format version";
    IMAGE_NO_CODE = 320, "image contains no code";
    IMAGE_INCOMPAT_HDR_FMT = 321, "incomptabile image file format: method header too old";
    UNAVAIL_INTRINSIC = 322, "unavailable intrinsic function called (index %d in function set \"%s\")";
    UNKNOWN_METACLASS_INTERNAL = 323, "unknown internal intrinsic class ID %x";
    XOR_MASK_BAD_IN_MEM = 324, "page mask is not allowed for in-memory image file";
    NO_IMAGE_IN_EXE = 325, "no embedded image file found in executable";
    OBJ_SIZE_OVERFLOW = 326, "object size exceeds hardware limits of this computer";
    METACLASS_TOO_OLD = 327, "this interpreter is too old to run this program (program requires intrinsic class version %s, interpreter provides version %s)";
    INVAL_METACLASS_DATA = 328, "invalid intrinsic class data - image file may be corrupted";
    BAD_STATIC_NEW = 329, "invalid object - class does not allow loading";
    FUNCSET_TOO_OLD = 330, "this interpreter is too old to run this program (program requires function set version %s, interpreter provides version %s)";
    INVAL_EXPORT_TYPE = 331, "exported symbol \"%s\" is of incorrect datatype";
    INVAL_IMAGE_MACRO = 332, "invalid data in macro definitions in image file (error code %d)";
    NO_MAINRESTORE = 333, "this program is not capable of restoring a saved state on startup";
    IMAGE_INCOMPAT_VSN_DBG = 334, "image file is incompatible with debugger - recompile the program";
    NETWORK_SAFETY = 400, "this operation is not allowed by the network safety level settings";
    INVALID_SETPROP = 1001, "property cannot be set for object";
    NOT_SAVED_STATE = 1201, "file is not a valid saved state file";
    WRONG_SAVED_STATE = 1202, "saved state is for a different program or a different version of this program";
    SAVED_META_TOO_LONG = 1203, "intrinsic class name in saved state file is too long";
    SAVED_OBJ_ID_INVALID = 1206, "invalid object ID in saved state file";
    BAD_SAVED_STATE = 1207, "saved state file is corrupted (incorrect checksum)";
    BAD_SAVED_META_DATA = 1208, "invalid intrinsic class data in saved state file";
    STORAGE_SERVER_ERR = 1209, "storage server error";
    DESC_TAB_OVERFLOW = 1210, "saved file metadata table exceeds 64k bytes";
    NO_STR_CONV = 2001, "cannot convert value to string";
    CONV_BUF_OVF = 2002, "string conversion buffer overflow";
    BAD_TYPE_ADD = 2003, "invalid datatypes for addition operator";
    NUM_VAL_REQD = 2004, "numeric value required";
    INT_VAL_REQD = 2005, "integer value required";
    NO_LOG_CONV = 2006, "cannot convert value to logical (true/nil)";
    BAD_TYPE_SUB = 2007, "invalid datatypes for subtraction operator";
    DIVIDE_BY_ZERO = 2008, "division by zero";
    INVALID_COMPARISON = 2009, "invalid comparison";
    OBJ_VAL_REQD = 2010, "object value required";
    PROPPTR_VAL_REQD = 2011, "property pointer required";
    LOG_VAL_REQD = 2012, "logical value required";
    FUNCPTR_VAL_REQD = 2013, "function pointer required";
    CANNOT_INDEX_TYPE = 2014, "invalid index operation - this type of value cannot be indexed";
    INDEX_OUT_OF_RANGE = 2015, "index out of range";
    BAD_METACLASS_INDEX = 2016, "invalid intrinsic class index";
    BAD_DYNAMIC_NEW = 2017, "invalid dynamic object creation (intrinsic class does not support NEW)";
    OBJ_VAL_REQD_SC = 2018, "object value required for base class";
    STRING_VAL_REQD = 2019, "string value required";
    LIST_VAL_REQD = 2020, "list value required";
    DICT_NO_CONST = 2021, "list or string reference found in dictionary (entry \"%s\") - this dictionary cannot be saved in the image file";
    INVAL_OBJ_TYPE = 2022, "invalid object type - cannot convert to required object type";
    NUM_OVERFLOW = 2023, "numeric overflow";
    BAD_TYPE_MUL = 2024, "invalid datatypes for multiplication operator";
    BAD_TYPE_DIV = 2025, "invalid datatypes for division operator";
    BAD_TYPE_NEG = 2026, "invalid datatype for arithmetic negation operator";
    OUT_OF_RANGE = 2027, "value is out of range";
    STR_TOO_LONG = 2028, "string is too long";
    LIST_TOO_LONG = 2029, "list too long";
    TREE_TOO_DEEP_EQ = 2030, "maximum equality test/hash recursion depth exceeded";
    NO_INT_CONV = 2031, "cannot convert value to integer";
    BAD_TYPE_MOD = 2032, "invalid datatype for modulo operator";
    BAD_TYPE_BIT_AND = 2033, "invalid datatype for bitwise AND operator";
    BAD_TYPE_BIT_OR = 2034, "invalid datatype for bitwise OR operator";
    BAD_TYPE_XOR = 2035, "invalid datatype for XOR operator";
    BAD_TYPE_SHL = 2036, "invalid datatype for left-shift operator '<<'";
    BAD_TYPE_ASHR = 2037, "invalid datatype for arithmetic right-shift operator '>>'";
    BAD_TYPE_BIT_NOT = 2038, "invalid datatype for bitwise NOT operator";
    CODEPTR_VAL_REQD = 2039, "code pointer value required";
    EXCEPTION_OBJ_REQD = 2040, "exception object required, but 'new' did not yield an object";
    NO_DOUBLE_CONV = 2041, "cannot convert value to native floating point";
    NO_NUM_CONV = 2042, "cannot convert value to a numeric type";
    BAD_TYPE_LSHR = 2043, "invalid datatype for logical right-shift operator '>>>'";
    WRONG_NUM_OF_ARGS = 2201, "wrong number of arguments";
    WRONG_NUM_OF_ARGS_CALLING = 2202, "argument mismatch calling %s - function definition is incorrect";
    NIL_DEREF = 2203, "nil object reference";
    MISSING_NAMED_ARG = 2204, "missing named argument '%s'";
    BAD_TYPE_CALL = 2205, "invalid type for call";
    NIL_SELF = 2206, "nil 'self' value is not allowed";
    CANNOT_CREATE_INST = 2270, "cannot create instance of object - object is not a class";
    ILLEGAL_NEW = 2271, "cannot create instance - class does not allow dynamic construction";
    INVALID_OPCODE = 2301, "invalid opcode - possible image file corruption";
    UNHANDLED_EXC = 2302, "unhandled exception";
    STACK_OVERFLOW = 2303, "stack overflow";
    BAD_TYPE_BIF = 2304, "invalid type for intrinsic function argument";
    SAY_IS_NOT_DEFINED = 2305, "default output function is not defined";
    BAD_VAL_BIF = 2306, "invalid value for intrinsic function argument";
    BREAKPOINT = 2307, "breakpoint encountered";
    CALLEXT_NOT_IMPL = 2308, "external function calls are not implemented in this version";
    INVALID_OPCODE_MOD = 2309, "invalid opcode modifier - possible image file corruption";
    NO_CHARMAP_FILE = 2310, "No mapping file available for local character set \"%.32s\"";
    UNHANDLED_EXC_PARAM = 2311, "Unhandled exception: %s";
    VM_EXC_PARAM = 2312, "VM Error: %s";
    VM_EXC_CODE = 2313, "VM Error: code %d";
    EXC_IN_STATIC_INIT = 2314, "Exception in static initializer for %s.%s: %s";
    INTCLS_GENERAL_ERROR = 2315, "intrinsic class exception: %s";
    STACK_OUT_OF_BOUNDS = 2316, "stack access is out of bounds";
    DBG_ABORT = 2391, "'abort' signal";
    DBG_RESTART = 2392, "'restart' signal";
    DBG_HALT = 2394, "debugger VM halt";
    DBG_INTERRUPT = 2395, "interrupted by user";
    NO_DEBUGGER = 2396, "no debugger available";
    BAD_FRAME = 2500, "invalid frame in debugger local/parameter evaluation";
    BAD_SPEC_EVAL = 2501, "invalid speculative expression";
    INVAL_DBG_EXPR = 2502, "invalid debugger expression";
    NO_IMAGE_DBG_INFO = 2503, "image file has no debugging information - recompile for debugging";
    BIGNUM_NO_REGS = 2600, "out of temporary floating point registers (calculation too complex)";
    NO_BIGNUM_CONV = 2601, "cannot convert value to BigNumber";
}
