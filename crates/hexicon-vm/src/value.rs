//! Values: what a stack slot, local variable, or property can hold.

use std::fmt;

/// An object ID: an index into the VM's object table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ObjId(pub u32);

impl ObjId {
    /// The ID that refers to no object.
    pub const INVALID: ObjId = ObjId(0);

    /// Whether this ID can refer to an object.
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl fmt::Display for ObjId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A property ID.
pub type PropId = u16;

/// The property ID that refers to no property.
pub const INVALID_PROP: PropId = 0;

/// The calling interface of a method implemented by the VM itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeDesc {
    /// Number of required arguments.
    pub min_args: u8,
    /// Number of optional arguments after the required ones.
    pub opt_args: u8,
    /// Whether any number of further arguments is accepted.
    pub varargs: bool,
}

impl NativeDesc {
    /// A method taking exactly `min_args` arguments.
    pub const fn fixed(min_args: u8) -> Self {
        Self {
            min_args,
            opt_args: 0,
            varargs: false,
        }
    }

    /// A method taking `min_args` to `min_args + opt_args` arguments.
    pub const fn optional(min_args: u8, opt_args: u8) -> Self {
        Self {
            min_args,
            opt_args,
            varargs: false,
        }
    }

    /// A method taking `min_args` or more arguments.
    pub const fn variable(min_args: u8) -> Self {
        Self {
            min_args,
            opt_args: 0,
            varargs: true,
        }
    }

    /// Whether a call with `argc` arguments fits this interface.
    pub const fn accepts(self, argc: u32) -> bool {
        argc >= self.min_args as u32
            && (self.varargs || argc <= self.min_args as u32 + self.opt_args as u32)
    }
}

/// A T3 value.
///
/// The variants mirror the reference VM's value types. Strings and lists
/// appear twice: as constants (an offset into the image's constant pool)
/// and as objects (an [`ObjId`] of a string or list object).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// `nil`: false, and the absence of a value.
    Nil,
    /// `true`.
    True,
    /// No value at all. Marks unused slots; programs never see it.
    Empty,
    /// A 32-bit integer.
    Int(i32),
    /// An object reference.
    Obj(ObjId),
    /// A property ID.
    Prop(PropId),
    /// A constant string, by constant pool offset.
    SString(u32),
    /// A constant string that displays itself when evaluated as a property.
    DString(u32),
    /// A constant list, by constant pool offset.
    List(u32),
    /// Method code, by code pool offset. Only property values have this
    /// type; evaluating the property calls the code.
    CodeOfs(u32),
    /// A function pointer, by code pool offset.
    FuncPtr(u32),
    /// An enumerator.
    Enum(u32),
    /// A pointer to a function the VM implements.
    BifPtr {
        /// Index of the function set in the image's dependency list.
        set: u16,
        /// Index of the function within the set.
        index: u16,
    },
    /// An object to invoke when the property holding it is evaluated.
    ObjX(ObjId),
    /// A VM function to call when the property holding it is evaluated.
    BifPtrX {
        /// Index of the function set in the image's dependency list.
        set: u16,
        /// Index of the function within the set.
        index: u16,
    },
    /// A method the VM implements, as found by a property lookup that asked
    /// not to call it.
    NativeCode(NativeDesc),
}

/// Type codes as programs see them through `dataType()` and as images
/// store them in data holders.
pub mod type_code {
    #![allow(missing_docs)]
    pub const NIL: i32 = 1;
    pub const TRUE: i32 = 2;
    pub const OBJ: i32 = 5;
    pub const PROP: i32 = 6;
    pub const INT: i32 = 7;
    pub const SSTRING: i32 = 8;
    pub const DSTRING: i32 = 9;
    pub const LIST: i32 = 10;
    pub const CODEOFS: i32 = 11;
    pub const FUNCPTR: i32 = 12;
    pub const EMPTY: i32 = 13;
    pub const NATIVE_CODE: i32 = 14;
    pub const ENUM: i32 = 15;
    pub const BIFPTR: i32 = 16;
    pub const OBJX: i32 = 17;
    pub const BIFPTRX: i32 = 18;
}

impl Value {
    /// Size of a value as images encode it: a type byte and four data bytes.
    pub const HOLDER_SIZE: usize = 5;

    /// `true` or `nil`.
    pub const fn logical(condition: bool) -> Self {
        if condition { Self::True } else { Self::Nil }
    }

    /// An object reference, or `nil` for the invalid ID.
    pub const fn obj_or_nil(id: ObjId) -> Self {
        if id.is_valid() {
            Self::Obj(id)
        } else {
            Self::Nil
        }
    }

    /// The object ID, if this is an object reference.
    pub const fn as_obj(self) -> Option<ObjId> {
        match self {
            Self::Obj(id) => Some(id),
            _ => None,
        }
    }

    /// The object ID, or the invalid ID for anything but an object.
    pub const fn obj_id(self) -> ObjId {
        match self {
            Self::Obj(id) => id,
            _ => ObjId::INVALID,
        }
    }

    /// Whether this is `nil`.
    pub const fn is_nil(self) -> bool {
        matches!(self, Self::Nil)
    }

    /// Whether evaluating a property with this value runs code.
    pub const fn is_invocable_prop(self) -> bool {
        matches!(
            self,
            Self::CodeOfs(_) | Self::ObjX(_) | Self::BifPtrX { .. }
        )
    }

    /// The type code of this value.
    pub const fn type_code(self) -> i32 {
        match self {
            Self::Nil => type_code::NIL,
            Self::True => type_code::TRUE,
            Self::Empty => type_code::EMPTY,
            Self::Int(_) => type_code::INT,
            Self::Obj(_) => type_code::OBJ,
            Self::Prop(_) => type_code::PROP,
            Self::SString(_) => type_code::SSTRING,
            Self::DString(_) => type_code::DSTRING,
            Self::List(_) => type_code::LIST,
            Self::CodeOfs(_) => type_code::CODEOFS,
            Self::FuncPtr(_) => type_code::FUNCPTR,
            Self::Enum(_) => type_code::ENUM,
            Self::BifPtr { .. } => type_code::BIFPTR,
            Self::ObjX(_) => type_code::OBJX,
            Self::BifPtrX { .. } => type_code::BIFPTRX,
            Self::NativeCode(_) => type_code::NATIVE_CODE,
        }
    }

    /// Decode a value from the five-byte form images use.
    ///
    /// Type codes that cannot appear in an image decode as `nil`.
    pub fn from_holder(b: &[u8]) -> Self {
        let u32_at = || u32::from_le_bytes([b[1], b[2], b[3], b[4]]);
        let u16_at = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
        match i32::from(b[0]) {
            type_code::TRUE => Self::True,
            type_code::OBJ => Self::obj_or_nil(ObjId(u32_at())),
            type_code::PROP => Self::Prop(u16_at(1)),
            type_code::INT => Self::Int(u32_at() as i32),
            type_code::SSTRING => Self::SString(u32_at()),
            type_code::DSTRING => Self::DString(u32_at()),
            type_code::LIST => Self::List(u32_at()),
            type_code::CODEOFS => Self::CodeOfs(u32_at()),
            type_code::FUNCPTR => Self::FuncPtr(u32_at()),
            type_code::EMPTY => Self::Empty,
            type_code::ENUM => Self::Enum(u32_at()),
            type_code::BIFPTR => Self::BifPtr {
                index: u16_at(1),
                set: u16_at(3),
            },
            type_code::OBJX => Self::ObjX(ObjId(u32_at())),
            type_code::BIFPTRX => Self::BifPtrX {
                index: u16_at(1),
                set: u16_at(3),
            },
            _ => Self::Nil,
        }
    }
}
