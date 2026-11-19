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
}

/// A constant in an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConstVal {
    pub value: Value,
    /// The value comes from a question about the program being compiled
    /// (`defined(x)`), not from the source saying it outright. A
    /// condition that is constant for that reason is not worth a warning.
    pub ctc: bool,
}

impl ConstVal {
    pub(crate) fn new(value: Value) -> Self {
        Self { value, ctc: false }
    }

    /// The value as a condition. Only nil and zero are false.
    pub(crate) fn is_true(&self) -> bool {
        !matches!(self.value, Value::Nil | Value::Int(0))
    }

    /// Replace the value by true or nil. The reference changes the
    /// value in place, so the note of where it came from stays.
    pub(crate) fn set_bool(&mut self, value: bool) {
        self.value = if value { Value::True } else { Value::Nil };
    }

    pub(crate) fn is_bool(&self) -> bool {
        matches!(self.value, Value::Nil | Value::True)
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
}

impl AsiOp {
    /// The operator as messages name it.
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Simple => "=",
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
        matches!(self, Self::Sym(_) | Self::Local(_))
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
    },
    While {
        cond: Expr,
        body: Option<Box<Stmt>>,
    },
    DoWhile {
        body: Option<Box<Stmt>>,
        cond: Expr,
    },
    Break(Option<Vec<u8>>),
    Continue(Option<Vec<u8>>),
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
    /// Another definition has replaced this one, and no code is
    /// generated for it.
    pub replaced: bool,
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
