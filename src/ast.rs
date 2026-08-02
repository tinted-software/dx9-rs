#[derive(Debug, Clone)]
pub enum DataType {
    Void,
    Bool,
    Float,
    Float2,
    Float3,
    Float4,
    Float4x4,
    Float3x3,
    Float4x3,
    Float3x4,
    Float2x2,
    Int,
    Int2,
    Int3,
    Int4,
    Half,
    Half2,
    Half3,
    Half4,
    Sampler,
    Sampler2D,
    UserType(String),
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub data_type: DataType,
    pub semantic: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<StructField>,
}

#[derive(Debug, Clone)]
pub enum RegisterType {
    ConstantFloat(usize),
    Sampler(usize),
}

#[derive(Debug, Clone)]
pub struct VariableDecl {
    pub name: String,
    pub data_type: DataType,
    pub is_const: bool,
    pub is_static: bool,
    pub register: Option<RegisterType>,
    pub initializer: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum Expr {
    LiteralFloat(f32),
    LiteralInt(i32),
    LiteralBool(bool),
    Variable(String),
    MemberAccess(Box<Expr>, String),
    BinaryOp(Box<Expr>, Op, Box<Expr>),
    Construct(DataType, Vec<Expr>),
    FunctionCall(String, Vec<Expr>),
    Cast(DataType, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    InitializerList(Vec<Expr>),
}

#[derive(Debug, Clone)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Assign,
    GreaterThan,
    LessThan,
    GreaterThanEqual,
    LessThanEqual,
    Equal,
    NotEqual,
    And,
    Or,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Return(Option<Expr>),
    VariableDecl(VariableDecl),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    Block(Vec<Stmt>),
    // for (init; cond; post) body — init may be a decl or expr
    For(Box<Stmt>, Option<Expr>, Option<Expr>, Box<Stmt>),
    While(Expr, Box<Stmt>),
    Break,
}

#[derive(Debug, Clone)]
pub struct FunctionParam {
    pub name: String,
    pub data_type: DataType,
    pub semantic: Option<String>,
    pub is_const: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub return_type: DataType,
    pub return_semantic: Option<String>,
    pub params: Vec<FunctionParam>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Definition {
    Struct(StructDef),
    Variable(VariableDecl),
    Function(FunctionDef),
    Preprocessor(String),
}

#[derive(Debug, Clone)]
pub struct ShaderAST {
    pub definitions: Vec<Definition>,
}
