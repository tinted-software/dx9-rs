use logos::Logos;

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n\f]+")] // Skip whitespace
#[logos(skip r"//.*")] // Skip line comments
#[logos(skip r"/\*[^*]*\*+(?:[^/*][^*]*\*+)*/")] // Skip block comments
pub enum Token {
    // Keywords
    #[token("struct")]
    Struct,
    #[token("const")]
    Const,
    #[token("static")]
    Static,
    #[token("uniform")]
    Uniform,
    #[token("register")]
    Register,
    #[token("return")]
    Return,
    #[token("void")]
    Void,
    #[token("bool")]
    Bool,
    #[token("float")]
    Float,
    #[token("float2")]
    Float2,
    #[token("float3")]
    Float3,
    #[token("float4")]
    Float4,
    #[token("float4x4")]
    Float4x4,
    #[token("float3x3")]
    Float3x3,
    #[token("float4x3")]
    Float4x3,
    #[token("float3x4")]
    Float3x4,
    #[token("float2x2")]
    Float2x2,
    #[token("int")]
    Int,
    #[token("int2")]
    Int2,
    #[token("int3")]
    Int3,
    #[token("int4")]
    Int4,
    #[token("half")]
    Half,
    #[token("half2")]
    Half2,
    #[token("half3")]
    Half3,
    #[token("half4")]
    Half4,
    #[token("sampler")]
    Sampler,
    #[token("sampler2D")]
    Sampler2D,
    #[token("sampler3D")]
    Sampler3D,
    #[token("samplerCUBE")]
    SamplerCUBE,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("for")]
    For,
    #[token("while")]
    While,
    #[token("break")]
    Break,

    // Identifiers
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Identifier(String),

    // Literals — HLSL allows 1.0f, .5, 1e-5, 1.5E+3f, 1f
    #[regex(r"([0-9]+\.[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?[fF]?|[0-9]+[eE][+-]?[0-9]+[fF]?|[0-9]+[fF]", |lex| {
        let s = lex.slice();
        let s = s.strip_suffix('f').or_else(|| s.strip_suffix('F')).unwrap_or(s);
        s.parse::<f32>().ok()
    })]
    FloatLiteral(f32),

    #[regex(r"[0-9]+", |lex| {
        let s = lex.slice();
        s.parse::<i32>().ok().or_else(|| {
            // Oversized integer constants (hashes etc.) — clamp into i32.
            s.parse::<i64>()
                .ok()
                .map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
        })
    })]
    IntLiteral(i32),

    // Preprocessor line
    #[regex(r"\#[^\n]*", |lex| lex.slice().to_string())]
    Preprocessor(String),

    // Punctuation & Operators
    #[token(";")]
    Semicolon,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token(".")]
    Dot,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("=")]
    Assign,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Mul,
    #[token("/")]
    Div,
    #[token("?")]
    Question,
    #[token(">")]
    GreaterThan,
    #[token("<")]
    LessThan,
    #[token(">=")]
    GreaterThanEqual,
    #[token("<=")]
    LessThanEqual,
    #[token("==")]
    Equal,
    #[token("!=")]
    NotEqual,
    #[token("!")]
    Not,
    #[token("&&")]
    And,
    #[token("||")]
    Or,
    #[token("+=")]
    AddAssign,
    #[token("-=")]
    SubAssign,
    #[token("*=")]
    MulAssign,
    #[token("/=")]
    DivAssign,
    #[token("++")]
    PlusPlus,
    #[token("--")]
    MinusMinus,
}
