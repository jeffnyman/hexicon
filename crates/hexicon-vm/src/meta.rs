//! The intrinsic classes ("metaclasses") the VM provides, and the table
//! that ties them to the ones an image asks for.

use crate::error::{ErrParam, VmError, VmResult, code};
use crate::value::{INVALID_PROP, ObjId, PropId};

macro_rules! metaclasses {
    ($( $variant:ident $name:literal $version:literal $parent:ident; )*) => {
        /// An intrinsic class the VM knows about.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[allow(missing_docs)]
        pub enum MetaKind {
            $( $variant, )*
        }

        impl MetaKind {
            /// Every intrinsic class, in a fixed order.
            pub const ALL: &'static [MetaKind] = &[ $( MetaKind::$variant, )* ];

            /// The class's name in images, without the version suffix.
            pub const fn name(self) -> &'static str {
                match self {
                    $( Self::$variant => $name, )*
                }
            }

            /// The newest version of the class this VM provides.
            pub const fn version(self) -> &'static str {
                match self {
                    $( Self::$variant => $version, )*
                }
            }

            /// The intrinsic class this one derives from.
            #[allow(unreachable_patterns)]
            pub const fn parent(self) -> Option<MetaKind> {
                match self {
                    Self::Root => None,
                    $( Self::$variant => Some(MetaKind::$parent), )*
                }
            }
        }
    };
}

// The names and versions are the reference VM's. `Root` lists itself as
// its parent only to fit the table; `parent()` reports that it has none.
metaclasses! {
    Root "root-object" "030004" Root;
    TadsObject "tads-object" "030005" Root;
    IntClassMod "int-class-mod" "030000" Root;
    IntrinsicClass "intrinsic-class" "030001" Root;
    String "string" "030008" Root;
    Collection "collection" "030000" Root;
    List "list" "030008" Collection;
    Vector "vector" "030005" Collection;
    LookupTable "lookuptable" "030003" Collection;
    WeakRefLookupTable "weakreflookuptable" "030001" LookupTable;
    Iterator "iterator" "030001" Root;
    IndexedIterator "indexed-iterator" "030000" Iterator;
    LookupTableIterator "lookuptable-iterator" "030000" Iterator;
    AnonFuncPtr "anon-func-ptr" "030000" Vector;
    BigNumber "bignumber" "030001" Root;
    ByteArray "bytearray" "030002" Root;
    CharacterSet "character-set" "030001" Root;
    Date "date" "030000" Root;
    Dictionary "dictionary2" "030001" Root;
    DynamicFunc "dynamic-func" "030000" Root;
    FileName "filename" "030000" Root;
    File "file" "030003" Root;
    StackFrameDesc "stack-frame-desc" "030000" Root;
    StackFrameRef "stack-frame-ref" "030000" Root;
    GrammarProduction "grammar-production" "030002" Root;
    HttpRequest "http-request" "030001" Root;
    HttpServer "http-server" "030000" Root;
    RexPattern "regex-pattern" "030000" Root;
    StringBuffer "stringbuffer" "030000" Root;
    StringComparator "string-comparator" "030000" Root;
    TemporaryFile "tempfile" "030000" Root;
    TimeZone "timezone" "030000" Root;
}

impl MetaKind {
    /// The class with a given name, ignoring any version suffix.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }
}

/// One intrinsic class as an image uses it.
#[derive(Debug, Clone)]
pub struct MetaEntry {
    /// Which class this is.
    pub kind: MetaKind,
    /// The name the image gives, with its version suffix. Saved states
    /// record it.
    #[allow(dead_code)]
    pub image_name: String,
    /// The property the image uses for each of the class's methods. Method
    /// numbers start at 1, so method `n` is at index `n - 1`.
    pub method_props: Vec<PropId>,
    /// The intrinsic class object that represents the class to the
    /// program, or the invalid ID until it is known.
    pub class_obj: ObjId,
    /// The lowest property in `method_props`.
    min_prop: PropId,
    /// Method number by property, offset by `min_prop`; zero for none.
    by_prop: Vec<u16>,
}

impl MetaEntry {
    fn new(kind: MetaKind, image_name: String, method_props: Vec<PropId>) -> Self {
        let used = method_props.iter().copied().filter(|&p| p != INVALID_PROP);
        let min_prop = used.clone().min().unwrap_or(INVALID_PROP);
        let max_prop = used.max().unwrap_or(INVALID_PROP);
        let mut by_prop = Vec::new();
        if min_prop != INVALID_PROP {
            by_prop = vec![0; (max_prop - min_prop) as usize + 1];
            for (index, &prop) in method_props.iter().enumerate() {
                if prop != INVALID_PROP {
                    by_prop[(prop - min_prop) as usize] = index as u16 + 1;
                }
            }
        }
        Self {
            kind,
            image_name,
            method_props,
            class_obj: ObjId::INVALID,
            min_prop,
            by_prop,
        }
    }

    /// The number of the method a property selects, or zero if the
    /// property is not one of the class's methods.
    pub fn method_index(&self, prop: PropId) -> u16 {
        if prop < self.min_prop {
            return 0;
        }
        self.by_prop
            .get((prop - self.min_prop) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// The property for a method number, or the invalid property.
    pub fn method_prop(&self, method: u16) -> PropId {
        match method {
            0 => INVALID_PROP,
            n => self
                .method_props
                .get(n as usize - 1)
                .copied()
                .unwrap_or(INVALID_PROP),
        }
    }
}

/// The intrinsic classes in use, in the order of the image's dependency
/// list. Byte code refers to classes by their index here.
#[derive(Debug, Clone, Default)]
pub struct MetaTable {
    entries: Vec<MetaEntry>,
    /// Table index by `MetaKind`, or `u16::MAX` for a class not in the table.
    by_kind: Vec<u16>,
}

/// Split `name/version` into its parts. A missing version reads as the
/// oldest.
fn split_version(id: &str) -> (&str, &str) {
    id.split_once('/').unwrap_or((id, "000000"))
}

impl MetaTable {
    /// An empty table.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            by_kind: vec![u16::MAX; MetaKind::ALL.len()],
        }
    }

    /// Add a class the image depends on.
    pub fn add(&mut self, image_name: &str, method_props: Vec<PropId>) -> VmResult<()> {
        let (name, version) = split_version(image_name);
        let Some(kind) = MetaKind::from_name(name) else {
            return Err(VmError::with(
                code::UNKNOWN_METACLASS,
                vec![ErrParam::Str(image_name.to_owned())],
            ));
        };
        if version > kind.version() {
            return Err(VmError::with(
                code::METACLASS_TOO_OLD,
                vec![
                    ErrParam::Str(image_name.to_owned()),
                    ErrParam::Str(kind.version().to_owned()),
                ],
            ));
        }
        self.push(kind, image_name.to_owned(), method_props);
        Ok(())
    }

    /// Add a class the VM needs whether or not the image mentions it.
    pub fn add_if_new(&mut self, kind: MetaKind) {
        if self.index_of(kind).is_none() {
            let name = format!("{}/{}", kind.name(), kind.version());
            self.push(kind, name, Vec::new());
        }
    }

    fn push(&mut self, kind: MetaKind, image_name: String, method_props: Vec<PropId>) {
        // If an image lists a class twice, the first entry stays the one
        // found by kind, as in the reference VM.
        if self.by_kind[kind as usize] == u16::MAX {
            self.by_kind[kind as usize] = self.entries.len() as u16;
        }
        self.entries
            .push(MetaEntry::new(kind, image_name, method_props));
    }

    /// Number of classes in the table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The entry at a table index.
    pub fn get(&self, index: usize) -> Option<&MetaEntry> {
        self.entries.get(index)
    }

    /// The entry at a table index, for modification.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut MetaEntry> {
        self.entries.get_mut(index)
    }

    /// The table index of a class, if the table has it.
    pub fn index_of(&self, kind: MetaKind) -> Option<usize> {
        match self.by_kind[kind as usize] {
            u16::MAX => None,
            index => Some(index as usize),
        }
    }

    /// The entry for a class, if the table has it.
    pub fn entry(&self, kind: MetaKind) -> Option<&MetaEntry> {
        self.index_of(kind).map(|index| &self.entries[index])
    }

    /// The number of the method of `kind` that a property selects, or zero.
    pub fn method_index(&self, kind: MetaKind, prop: PropId) -> u16 {
        self.entry(kind).map_or(0, |entry| entry.method_index(prop))
    }

    /// The intrinsic class object for a class, or the invalid ID.
    pub fn class_obj(&self, kind: MetaKind) -> ObjId {
        self.entry(kind).map_or(ObjId::INVALID, |e| e.class_obj)
    }
}
