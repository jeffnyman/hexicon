//! The syntax tree: what the parser makes of the tokens, and what the
//! code generator reads.
//!
//! The tree follows the reference compiler's. That matters because the
//! byte code is generated node by node, and the choice of instructions
//! depends on what kind of node each operand is.

/// A local variable or parameter: an index into the program's list of
/// them.
pub(crate) type LocalId = usize;
/// A table of local variables for one scope.
pub(crate) type TabId = usize;

/// A value known when the program is compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Nil,
    True,
    Int(i32),
    /// A string, as its bytes of UTF-8.
    SStr(Vec<u8>),
    /// The address of a function: an index into the program's list of
    /// functions.
    FuncPtr(usize),
    /// A list all of whose elements are constants.
    List(Vec<ConstVal>),
    /// An object, by its number.
    Obj(u32),
}

/// A constant in an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConstVal {
    pub value: Value,
    /// The value comes from a question about the program being compiled
    /// (`defined(x)`), not from the source saying it outright. A
    /// condition that is constant for that reason is not worth a warning.
    pub ctc: bool,
    /// The elements of the list this constant used to be. The reference
    /// folds some operators by overwriting the value of the left
    /// operand in place, and when that operand is a list, what is left
    /// is a list that claims to be `true` or `nil`: everything that
    /// asks for its value sees the truth value, but the code generated
    /// for it pushes the list. `[1] == [1]` is `[1]`.
    pub list_node: Option<Vec<ConstVal>>,
}

impl ConstVal {
    pub(crate) fn new(value: Value) -> Self {
        Self {
            value,
            ctc: false,
            list_node: None,
        }
    }

    /// The value as a condition. Only nil and zero are false.
    pub(crate) fn is_true(&self) -> bool {
        !matches!(self.value, Value::Nil | Value::Int(0))
    }

    /// Replace the value by true or nil. The reference changes the
    /// value in place, so the note of where it came from stays.
    pub(crate) fn set_bool(&mut self, value: bool) {
        let new = if value { Value::True } else { Value::Nil };
        if let Value::List(elements) = std::mem::replace(&mut self.value, new) {
            self.list_node = Some(elements);
        }
    }

    /// Whether the value is sure to be true or nil when the program
    /// runs.
    pub(crate) fn is_bool(&self) -> bool {
        matches!(self.value, Value::Nil | Value::True) && self.list_node.is_none()
    }
}

/// An operator with one operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Not,
    /// Make a value true or nil. The parser adds this where `&&` or `||`
    /// is folded away but its result must still be a truth value.
    Boolize,
    BNot,
    Pos,
    Neg,
}

/// An operator with two operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Comma,
    Or,
    And,
    BOr,
    BAnd,
    BXor,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Shl,
    AShr,
    LShr,
    Mul,
    Div,
    Mod,
    Sub,
    Add,
}

/// A kind of assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AsiOp {
    Simple,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BAnd,
    BOr,
    BXor,
    Shl,
    AShr,
    LShr,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
    /// Not an assignment to the target itself but to an element of it:
    /// the target is what is being indexed in `target[i] = value`.
    Idx,
}

impl AsiOp {
    /// The operator as messages name it.
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Simple | Self::Idx => "=",
            Self::Add => "+=",
            Self::Sub => "-=",
            Self::Mul => "*=",
            Self::Div => "/=",
            Self::Mod => "%=",
            Self::BAnd => "&=",
            Self::BOr => "|=",
            Self::BXor => "^=",
            Self::Shl => "<<=",
            Self::AShr => ">>=",
            Self::LShr => ">>>=",
            Self::PreInc | Self::PostInc => "++",
            Self::PreDec | Self::PostDec => "--",
        }
    }
}

/// One argument of a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Arg {
    pub expr: Expr,
    /// The argument is a list to be passed as that many arguments
    /// (`f(lst...)`).
    pub varargs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expr {
    Const(ConstVal),
    /// A name that is not a local variable. What it is cannot be known
    /// until the whole program has been read.
    Sym(Vec<u8>),
    Local(LocalId),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// An assignment of any kind but `++` and `--`.
    Assign(AsiOp, Box<Expr>, Box<Expr>),
    /// `++` or `--`, before or after.
    IncDec(AsiOp, Box<Expr>),
    /// `a ? b : c`
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `a ?? b`
    IfNil(Box<Expr>, Box<Expr>),
    /// A call, with its arguments in the order the source has them.
    Call(Box<Expr>, Vec<Arg>),
    /// A list with an element that is not a constant.
    List(Vec<Expr>),
    /// `a[b]`
    Index(Box<Expr>, Box<Expr>),
    /// A string in double quotes: text to display.
    DStr(Vec<u8>),
    /// An expression embedded in a string in double quotes: its value
    /// is displayed.
    DStrEmbed(Box<Expr>),
    /// `<<one of>>` in a string: the alternatives. The flag is for a
    /// string in double quotes.
    OneOf(bool, Vec<Expr>),
    /// `self`
    SelfRef,
    /// `obj.prop`, or with arguments `obj.prop(args)`. The property is
    /// a name, or with `prop_is_expr` an expression in parentheses
    /// whose value says which property.
    Member {
        obj: Box<Expr>,
        prop: Box<Expr>,
        prop_is_expr: bool,
        args: Option<Vec<Arg>>,
    },
}

impl Expr {
    pub(crate) fn constant(value: Value) -> Self {
        Self::Const(ConstVal::new(value))
    }

    pub(crate) fn as_const(&self) -> Option<&ConstVal> {
        match self {
            Self::Const(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn is_const(&self) -> bool {
        self.as_const().is_some()
    }

    pub(crate) fn is_const_int(&self, int: i32) -> bool {
        matches!(self.as_const(), Some(ConstVal { value: Value::Int(value), .. }) if *value == int)
    }

    /// Whether the value is sure to be true or nil.
    pub(crate) fn is_bool(&self) -> bool {
        match self {
            Self::Const(value) => value.is_bool(),
            Self::Unary(UnOp::Not, _) => true,
            Self::Binary(op, _, right) => match op {
                BinOp::Comma => right.is_bool(),
                BinOp::Eq
                | BinOp::Ne
                | BinOp::And
                | BinOp::Or
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::Lt
                | BinOp::Le => true,
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether the expression can be assigned to, as far as the parser
    /// can tell.
    pub(crate) fn check_lvalue(&self) -> bool {
        matches!(
            self,
            Self::Sym(_) | Self::Local(_) | Self::Index(..) | Self::Member { args: None, .. }
        )
    }

    /// A list of these elements: a constant if they all are.
    pub(crate) fn list(elements: Vec<Expr>) -> Self {
        if elements.iter().all(Self::is_const) {
            let values = elements
                .into_iter()
                .map(|element| match element {
                    Self::Const(value) => value,
                    _ => unreachable!("checked to be constants"),
                })
                .collect();
            Self::constant(Value::List(values))
        } else {
            Self::List(elements)
        }
    }

    /// How many elements a constant list has, if that is what this is.
    pub(crate) fn const_list_len(&self) -> Option<usize> {
        match self {
            Self::Const(ConstVal {
                value: Value::List(elements),
                ..
            }) => Some(elements.len()),
            _ => None,
        }
    }
}

/// A statement, and where in the source it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stmt {
    pub kind: StmtKind,
    pub file: usize,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StmtKind {
    /// Nothing: what an empty block holds.
    Null,
    /// The body of the constructor the compiler gives an object with
    /// several superclasses and no constructor of its own: it passes
    /// its arguments to each superclass's constructor. These are the
    /// superclasses' numbers.
    ImplicitCtor(Vec<u32>),
    /// A block.
    Comp {
        stmts: Vec<Stmt>,
        /// The table of locals in force in the block.
        symtab: Option<TabId>,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then_part: Option<Box<Stmt>>,
        else_part: Option<Box<Stmt>>,
    },
    Return(Option<Expr>),
    For {
        init: Option<Expr>,
        cond: Option<Expr>,
        reinit: Option<Expr>,
        body: Option<Box<Stmt>>,
        symtab: Option<TabId>,
        id: usize,
    },
    While {
        cond: Expr,
        body: Option<Box<Stmt>>,
        id: usize,
    },
    DoWhile {
        body: Option<Box<Stmt>>,
        cond: Expr,
        id: usize,
    },
    Break(Option<Vec<u8>>),
    Continue(Option<Vec<u8>>),
    Switch {
        expr: Expr,
        body: Box<Stmt>,
        /// How many `case` labels the parser counted in the body.
        case_cnt: usize,
        has_default: bool,
        id: usize,
    },
    Case {
        expr: Expr,
        stm: Option<Box<Stmt>>,
    },
    Default(Option<Box<Stmt>>),
    /// A statement with a label on it.
    Label {
        /// The label, as an index into [`CodeBody::labels`]. There is
        /// none when the name was already taken by another statement.
        label: Option<usize>,
        stm: Option<Box<Stmt>>,
        id: usize,
    },
    Goto(Vec<u8>),
    Try {
        id: usize,
        body: Option<Box<Stmt>>,
        catches: Vec<Catch>,
        finally: Option<Box<Finally>>,
    },
    Throw(Expr),
}

/// A statement that has other statements inside it and that matters to
/// a jump out of them or into them: a loop, a `switch`, a labeled
/// statement, a `try`, or a `finally` block. Each has a number of its
/// own within the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Enclosure {
    pub id: usize,
    pub is_finally: bool,
}

/// A `catch` clause of a `try`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Catch {
    /// The name of the class of exception it catches.
    pub class: Vec<u8>,
    /// The local the exception is put in.
    pub var: LocalId,
    pub body: Option<Box<Stmt>>,
    pub symtab: Option<TabId>,
    pub file: usize,
    pub line: u32,
}

/// The `finally` clause of a `try`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finally {
    pub id: usize,
    pub body: Option<Box<Stmt>>,
    /// The locals that hold an exception while the block runs, and the
    /// place to go back to when it is done.
    pub exc_local: usize,
    pub jsr_local: usize,
    pub file: usize,
    pub line: u32,
}

/// A label in a function, for `goto`, `break`, and `continue`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LabelSym {
    pub name: Vec<u8>,
    /// How the label is used: the `GOTO`, `BREAK`, and `CONT` bits of
    /// [`flow`], set as the uses are found.
    pub flags: std::cell::Cell<u32>,
    /// Where the labeled statement is.
    pub file: usize,
    pub line: u32,
    /// The statements the labeled statement is inside, as the parser
    /// saw it, outermost first and ending with the labeled statement
    /// itself. (The parser takes a `catch` block to be outside its
    /// `try`; the code generator takes it to be inside.)
    pub within: Vec<Enclosure>,
}

impl Stmt {
    /// Whether the statement starts with a label, a `case`, or a
    /// `default`, so that it can be reached from somewhere other than
    /// the statement before it.
    pub(crate) fn has_code_label(&self) -> bool {
        match &self.kind {
            StmtKind::Label { .. } | StmtKind::Case { .. } | StmtKind::Default(_) => true,
            StmtKind::Comp { stmts, .. } => stmts.first().is_some_and(Stmt::has_code_label),
            _ => false,
        }
    }
}

/// The body of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeBody {
    /// The outermost table of locals: the parameters, and the locals of
    /// the outermost block.
    pub lcltab: Option<TabId>,
    pub stm: Stmt,
    pub argc: usize,
    pub opt_argc: usize,
    pub varargs: bool,
    /// The parameter that collects the variable arguments in a list
    /// (`f(a, [rest])`), if there is one.
    pub varargs_list_local: Option<LocalId>,
    pub local_cnt: usize,
    /// The code is a method's: it has a `self`.
    pub self_valid: bool,
    /// Another definition has replaced this one, and no code is
    /// generated for it.
    pub replaced: bool,
    /// The labels on statements in the body.
    pub labels: std::rc::Rc<Vec<LabelSym>>,
    /// Where the closing brace is.
    pub end_file: usize,
    pub end_line: u32,
}

/// The ways a statement can be left.
pub(crate) mod flow {
    pub(crate) const NEXT: u32 = 0x01;
    pub(crate) const THROW: u32 = 0x02;
    pub(crate) const RET_VAL: u32 = 0x04;
    pub(crate) const RET_VOID: u32 = 0x08;
    pub(crate) const BREAK: u32 = 0x10;
    pub(crate) const CONT: u32 = 0x20;
    pub(crate) const GOTO: u32 = 0x40;
}
