use crate::ast::*;
use crate::lexer::Token;
use annotate_snippets::{AnnotationKind, Level, Renderer, Snippet};
use logos::Logos;
use std::ops::Range;

pub struct Parser<'a> {
    source: &'a str,
    tokens: Vec<(Token, Range<usize>)>,
    pos: usize,
    filepath: &'a str,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str, filepath: &'a str) -> Self {
        let lexer = Token::lexer(source);
        let mut tokens = Vec::new();

        for (token_res, span) in lexer.spanned() {
            if let Ok(token) = token_res {
                tokens.push((token, span));
            } else {
                // We could report lexical errors here, but for simplicity, we skip or handle them.
            }
        }

        Self {
            source,
            tokens,
            pos: 0,
            filepath,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    fn peek_span(&self) -> Range<usize> {
        self.tokens
            .get(self.pos)
            .map(|(_, s)| s.clone())
            .unwrap_or(self.source.len()..self.source.len())
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let (token, _) = &self.tokens[self.pos];
            self.pos += 1;
            Some(token.clone())
        } else {
            None
        }
    }

    fn error(&self, message: &str, span: Range<usize>, label: &str) -> ! {
        let message_str = message.to_string();
        let label_str = label.to_string();

        let report = Level::ERROR.primary_title(&message_str).element(
            Snippet::source(self.source)
                .line_start(1)
                .path(self.filepath)
                .annotation(AnnotationKind::Primary.span(span).label(&label_str)),
        );

        let renderer = Renderer::styled();
        eprintln!("{}", renderer.render(&[report]));
        panic!("{}", message_str);
    }

    fn expect(&mut self, expected: Token, err_msg: &str) -> Range<usize> {
        let span = self.peek_span();
        if let Some(t) = self.peek()
            && *t == expected
        {
            self.advance();
            return span;
        }
        self.error(err_msg, span, &format!("expected {:?}", expected));
    }

    pub fn parse(&mut self) -> ShaderAST {
        self.try_parse().unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible entry point for FFI — never unwinds past this frame on parse errors.
    pub fn try_parse(&mut self) -> Result<ShaderAST, String> {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut definitions = Vec::new();
            while self.pos < self.tokens.len() {
                if let Some(Token::Preprocessor(content)) = self.peek() {
                    definitions.push(Definition::Preprocessor(content.clone()));
                    self.advance();
                    continue;
                }
                definitions.push(self.parse_definition());
            }
            ShaderAST { definitions }
        }));
        std::panic::set_hook(prev_hook);
        result.map_err(|payload| {
            if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else {
                "parse error".to_string()
            }
        })
    }

    fn parse_definition(&mut self) -> Definition {
        let mut is_const = false;
        let mut is_static = false;

        while self.peek() == Some(&Token::Const) || self.peek() == Some(&Token::Static) {
            if self.peek() == Some(&Token::Const) {
                self.advance();
                is_const = true;
            } else if self.peek() == Some(&Token::Static) {
                self.advance();
                is_static = true;
            }
        }

        if self.peek() == Some(&Token::Struct) {
            if is_const || is_static {
                self.error(
                    "Struct cannot be const or static",
                    self.peek_span(),
                    "invalid modifiers",
                );
            }
            return Definition::Struct(self.parse_struct_def());
        }

        // Type
        let data_type = self.parse_data_type();

        // Name
        let name_span = self.peek_span();
        let name = match self.advance() {
            Some(Token::Identifier(id)) => id,
            _ => self.error(
                "Expected identifier for definition name",
                name_span,
                "expected name here",
            ),
        };

        let _is_array = if self.peek() == Some(&Token::LBracket) {
            self.advance();
            self.advance(); // consume the size token
            self.expect(Token::RBracket, "Expected ']' after array size");
            true
        } else {
            false
        };

        if self.peek() == Some(&Token::LParen) {
            if is_const || is_static {
                self.error(
                    "Function cannot be const or static",
                    name_span,
                    "invalid modifiers",
                );
            }
            return Definition::Function(self.parse_function_def(data_type, name));
        }

        // Variable declaration
        let register = if self.peek() == Some(&Token::Colon) {
            self.advance();
            self.expect(Token::Register, "Expected 'register' keyword after colon");
            self.expect(Token::LParen, "Expected '(' after register");
            let mut reg_val = None;
            while self.peek() != Some(&Token::RParen) && self.pos < self.tokens.len() {
                let id_span = self.peek_span();
                match self.peek() {
                    Some(Token::Identifier(id)) => {
                        let id_clone = id.clone();
                        self.advance();
                        let prefix = id_clone.chars().next().unwrap_or(' ').to_ascii_lowercase();
                        let num_str: String = id_clone.chars().skip(1).collect();
                        if let Ok(num) = num_str.parse::<usize>() {
                            if prefix == 'c' {
                                reg_val = Some(RegisterType::ConstantFloat(num));
                            } else if prefix == 's' {
                                reg_val = Some(RegisterType::Sampler(num));
                            }
                        }
                    }
                    _ => {
                        self.advance();
                    }
                }
            }
            self.expect(Token::RParen, "Expected ')' after register");
            Some(reg_val.unwrap_or(RegisterType::ConstantFloat(0)))
        } else {
            None
        };

        let initializer = if self.peek() == Some(&Token::Assign) {
            self.advance();
            Some(self.parse_expr())
        } else {
            None
        };

        self.expect(Token::Semicolon, "Expected ';' after variable declaration");

        Definition::Variable(VariableDecl {
            name,
            data_type,
            is_const,
            is_static,
            register,
            initializer,
        })
    }

    fn parse_data_type(&mut self) -> DataType {
        let span = self.peek_span();
        match self.advance() {
            Some(Token::Void) => DataType::Void,
            Some(Token::Bool) => DataType::Bool,
            Some(Token::Float) => DataType::Float,
            Some(Token::Float2) => DataType::Float2,
            Some(Token::Float3) => DataType::Float3,
            Some(Token::Float4) => DataType::Float4,
            Some(Token::Float4x4) => DataType::Float4x4,
            Some(Token::Float3x3) => DataType::Float3x3,
            Some(Token::Float4x3) => DataType::Float4x3,
            Some(Token::Float3x4) => DataType::Float3x4,
            Some(Token::Float2x2) => DataType::Float2x2,
            Some(Token::Int) => DataType::Int,
            Some(Token::Int2) => DataType::Int2,
            Some(Token::Int3) => DataType::Int3,
            Some(Token::Int4) => DataType::Int4,
            Some(Token::Half) => DataType::Half,
            Some(Token::Half2) => DataType::Half2,
            Some(Token::Half3) => DataType::Half3,
            Some(Token::Half4) => DataType::Half4,
            Some(Token::Sampler) => DataType::Sampler,
            Some(Token::Sampler2D) => DataType::Sampler2D,
            Some(Token::Sampler3D) => DataType::Sampler3D,
            Some(Token::SamplerCUBE) => DataType::SamplerCUBE,
            Some(Token::Identifier(id)) => DataType::UserType(id),
            _ => self.error("Expected data type", span, "invalid type"),
        }
    }

    fn parse_struct_def(&mut self) -> StructDef {
        self.expect(Token::Struct, "Expected 'struct'");
        let name_span = self.peek_span();
        let name = match self.advance() {
            Some(Token::Identifier(id)) => id,
            _ => self.error(
                "Expected struct name",
                name_span,
                "expected struct name here",
            ),
        };
        self.expect(Token::LBrace, "Expected '{' to start struct body");

        let mut fields = Vec::new();
        while self.peek() != Some(&Token::RBrace) {
            let data_type = self.parse_data_type();
            let field_name_span = self.peek_span();
            let field_name = match self.advance() {
                Some(Token::Identifier(id)) => id,
                _ => self.error(
                    "Expected field name",
                    field_name_span,
                    "expected field name here",
                ),
            };

            let semantic = if self.peek() == Some(&Token::Colon) {
                self.advance();
                let sem_span = self.peek_span();
                match self.advance() {
                    Some(Token::Identifier(sem)) => Some(sem),
                    _ => self.error(
                        "Expected semantic name after colon",
                        sem_span,
                        "invalid semantic",
                    ),
                }
            } else {
                None
            };

            self.expect(Token::Semicolon, "Expected ';' after struct field");
            fields.push(StructField {
                name: field_name,
                data_type,
                semantic,
            });
        }

        self.expect(Token::RBrace, "Expected '}' to end struct body");
        self.expect(Token::Semicolon, "Expected ';' after struct definition");

        StructDef { name, fields }
    }

    fn parse_function_def(&mut self, return_type: DataType, name: String) -> FunctionDef {
        self.expect(Token::LParen, "Expected '(' for parameter list");
        let mut params = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                // Skip parameter modifiers: const / uniform / in / out / inout
                loop {
                    match self.peek() {
                        Some(Token::Const) | Some(Token::Uniform) => {
                            self.advance();
                        }
                        Some(Token::Identifier(id))
                            if id == "in" || id == "out" || id == "inout" || id == "uniform" =>
                        {
                            self.advance();
                        }
                        _ => break,
                    }
                }
                let is_const = false;
                let data_type = self.parse_data_type();
                let param_name_span = self.peek_span();
                let param_name = match self.advance() {
                    Some(Token::Identifier(id)) => id,
                    _ => self.error(
                        "Expected parameter name",
                        param_name_span,
                        "expected parameter name here",
                    ),
                };

                // HLSL array params: `float3 cAmbientCube[6]`
                if self.peek() == Some(&Token::LBracket) {
                    self.advance();
                    self.advance(); // size (literal or identifier)
                    self.expect(Token::RBracket, "Expected ']' after array size");
                }

                let semantic = if self.peek() == Some(&Token::Colon) {
                    self.advance();
                    let sem_span = self.peek_span();
                    match self.advance() {
                        Some(Token::Identifier(sem)) => Some(sem),
                        _ => self.error(
                            "Expected semantic after colon",
                            sem_span,
                            "invalid semantic",
                        ),
                    }
                } else {
                    None
                };

                params.push(FunctionParam {
                    name: param_name,
                    data_type,
                    semantic,
                    is_const,
                });

                // HLSL default arguments: `bool b = false`, `float z = 1.0f`
                if self.peek() == Some(&Token::Assign) {
                    self.advance();
                    let _default = self.parse_expr();
                }

                if self.peek() == Some(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen, "Expected ')' to close parameter list");

        let return_semantic = if self.peek() == Some(&Token::Colon) {
            self.advance();
            let sem_span = self.peek_span();
            match self.advance() {
                Some(Token::Identifier(sem)) => Some(sem),
                _ => self.error(
                    "Expected return semantic after colon",
                    sem_span,
                    "invalid semantic",
                ),
            }
        } else {
            None
        };

        self.expect(Token::LBrace, "Expected '{' to start function body");
        let mut body = Vec::new();
        while self.peek() != Some(&Token::RBrace) {
            body.push(self.parse_stmt());
        }
        self.expect(Token::RBrace, "Expected '}' to close function body");

        FunctionDef {
            name,
            return_type,
            return_semantic,
            params,
            body,
        }
    }

    fn parse_stmt(&mut self) -> Stmt {
        let _span = self.peek_span();
        match self.peek() {
            Some(Token::Return) => {
                self.advance();
                let expr = if self.peek() != Some(&Token::Semicolon) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                self.expect(Token::Semicolon, "Expected ';' after return statement");
                Stmt::Return(expr)
            }
            Some(Token::LBrace) => {
                self.advance();
                let mut stmts = Vec::new();
                while self.peek() != Some(&Token::RBrace) {
                    stmts.push(self.parse_stmt());
                }
                self.advance(); // consume RBrace
                Stmt::Block(stmts)
            }
            Some(Token::If) => {
                self.advance();
                self.expect(Token::LParen, "Expected '(' after 'if'");
                let cond = self.parse_expr();
                self.expect(Token::RParen, "Expected ')' after if condition");
                let then_branch = self.parse_stmt();
                let else_branch = if self.peek() == Some(&Token::Else) {
                    self.advance();
                    Some(Box::new(self.parse_stmt()))
                } else {
                    None
                };
                Stmt::If(cond, Box::new(then_branch), else_branch)
            }
            Some(Token::For) => {
                self.advance();
                self.expect(Token::LParen, "Expected '(' after 'for'");
                // Init part: variable decl or expression or empty
                let init = if self.peek() == Some(&Token::Semicolon) {
                    self.advance();
                    Stmt::Expr(Expr::LiteralInt(0)) // empty init
                } else {
                    self.parse_stmt() // parses decl or expr; consumes ;
                };
                // Condition
                let cond = if self.peek() == Some(&Token::Semicolon) {
                    self.advance();
                    None
                } else {
                    let e = self.parse_expr();
                    self.expect(Token::Semicolon, "Expected ';' after for condition");
                    Some(e)
                };
                // Post expression
                let post = if self.peek() == Some(&Token::RParen) {
                    None
                } else {
                    let e = self.parse_expr();
                    Some(e)
                };
                self.expect(Token::RParen, "Expected ')' after for clauses");
                let body = self.parse_stmt();
                Stmt::For(Box::new(init), cond, post, Box::new(body))
            }
            Some(Token::While) => {
                self.advance();
                self.expect(Token::LParen, "Expected '(' after 'while'");
                let cond = self.parse_expr();
                self.expect(Token::RParen, "Expected ')' after while condition");
                let body = self.parse_stmt();
                Stmt::While(cond, Box::new(body))
            }
            Some(Token::Break) => {
                self.advance();
                self.expect(Token::Semicolon, "Expected ';' after break");
                Stmt::Break
            }
            _ => {
                // Could be expression or variable declaration.
                // Let's check modifiers
                let mut is_const = false;
                let mut is_static = false;

                while self.peek() == Some(&Token::Const) || self.peek() == Some(&Token::Static) {
                    if self.peek() == Some(&Token::Const) {
                        self.advance();
                        is_const = true;
                    } else if self.peek() == Some(&Token::Static) {
                        self.advance();
                        is_static = true;
                    }
                }

                // If it's a known type or a user identifier followed by another identifier (e.g. VS_OUTPUT o = ...)
                // We parse as variable declaration.
                let is_decl = match self.peek() {
                    Some(Token::Float)
                    | Some(Token::Float2)
                    | Some(Token::Float3)
                    | Some(Token::Float4)
                    | Some(Token::Float4x4)
                    | Some(Token::Float3x3)
                    | Some(Token::Float4x3)
                    | Some(Token::Float3x4)
                    | Some(Token::Float2x2)
                    | Some(Token::Int)
                    | Some(Token::Int2)
                    | Some(Token::Int3)
                    | Some(Token::Int4)
                    | Some(Token::Half)
                    | Some(Token::Half2)
                    | Some(Token::Half3)
                    | Some(Token::Half4)
                    | Some(Token::Bool) => true,
                    Some(Token::Identifier(_)) => {
                        // Lookahead to see if next is identifier
                        if self.pos + 1 < self.tokens.len() {
                            matches!(self.tokens[self.pos + 1].0, Token::Identifier(_))
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                if is_decl || is_const || is_static {
                    let data_type = self.parse_data_type();
                    let var_name_span = self.peek_span();
                    let name = match self.advance() {
                        Some(Token::Identifier(id)) => id,
                        _ => self.error("Expected variable name", var_name_span, "expected name"),
                    };
                    let _is_array = if self.peek() == Some(&Token::LBracket) {
                        self.advance();
                        self.advance(); // consume the size token
                        self.expect(Token::RBracket, "Expected ']' after array size");
                        true
                    } else {
                        false
                    };
                    let initializer = if self.peek() == Some(&Token::Assign) {
                        self.advance();
                        Some(self.parse_expr())
                    } else {
                        None
                    };
                    let mut decls = vec![Stmt::VariableDecl(VariableDecl {
                        name,
                        data_type: data_type.clone(),
                        is_const,
                        is_static,
                        register: None,
                        initializer,
                    })];
                    // Support: float3 a, b, c;
                    while self.peek() == Some(&Token::Comma) {
                        self.advance();
                        let extra_span = self.peek_span();
                        let extra_name = match self.advance() {
                            Some(Token::Identifier(id)) => id,
                            _ => self.error(
                                "Expected variable name after ','",
                                extra_span,
                                "expected name",
                            ),
                        };
                        // Optional array brackets
                        if self.peek() == Some(&Token::LBracket) {
                            self.advance();
                            self.advance();
                            self.expect(Token::RBracket, "Expected ']' after array size");
                        }
                        let extra_init = if self.peek() == Some(&Token::Assign) {
                            self.advance();
                            Some(self.parse_expr())
                        } else {
                            None
                        };
                        decls.push(Stmt::VariableDecl(VariableDecl {
                            name: extra_name,
                            data_type: data_type.clone(),
                            is_const,
                            is_static,
                            register: None,
                            initializer: extra_init,
                        }));
                    }
                    self.expect(
                        Token::Semicolon,
                        "Expected ';' after local variable declaration",
                    );
                    if decls.len() == 1 {
                        decls.remove(0)
                    } else {
                        Stmt::Block(decls)
                    }
                } else {
                    let expr = self.parse_expr();
                    self.expect(Token::Semicolon, "Expected ';' after expression");
                    Stmt::Expr(expr)
                }
            }
        }
    }

    fn parse_expr(&mut self) -> Expr {
        let cond = self.parse_binary_expr(0);
        if self.peek() == Some(&Token::Question) {
            self.advance(); // consume '?'
            let then_expr = self.parse_expr();
            self.expect(Token::Colon, "Expected ':' in conditional expression");
            let else_expr = self.parse_expr();
            Expr::Ternary(Box::new(cond), Box::new(then_expr), Box::new(else_expr))
        } else {
            cond
        }
    }

    fn parse_binary_expr(&mut self, min_precedence: u8) -> Expr {
        let mut lhs = self.parse_primary_expr();

        while let Some(op) = self.peek_op() {
            let prec = op_precedence(&op);
            if prec < min_precedence {
                break;
            }
            self.advance(); // consume op
            let rhs = self.parse_binary_expr(prec + 1);
            lhs = Expr::BinaryOp(Box::new(lhs), op, Box::new(rhs));
        }

        lhs
    }

    fn peek_op(&self) -> Option<Op> {
        match self.peek() {
            Some(Token::Plus) => Some(Op::Add),
            Some(Token::Minus) => Some(Op::Sub),
            Some(Token::Mul) => Some(Op::Mul),
            Some(Token::Div) => Some(Op::Div),
            Some(Token::Assign) => Some(Op::Assign),
            Some(Token::GreaterThan) => Some(Op::GreaterThan),
            Some(Token::LessThan) => Some(Op::LessThan),
            Some(Token::GreaterThanEqual) => Some(Op::GreaterThanEqual),
            Some(Token::LessThanEqual) => Some(Op::LessThanEqual),
            Some(Token::Equal) => Some(Op::Equal),
            Some(Token::NotEqual) => Some(Op::NotEqual),
            Some(Token::And) => Some(Op::And),
            Some(Token::Or) => Some(Op::Or),
            Some(Token::AddAssign) => Some(Op::AddAssign),
            Some(Token::SubAssign) => Some(Op::SubAssign),
            Some(Token::MulAssign) => Some(Op::MulAssign),
            Some(Token::DivAssign) => Some(Op::DivAssign),
            _ => None,
        }
    }

    fn parse_primary_expr(&mut self) -> Expr {
        // Unary +/-/! and prefix ++/--
        if self.peek() == Some(&Token::Minus) {
            self.advance(); // consume '-'
            let expr = self.parse_primary_expr();
            return Expr::BinaryOp(Box::new(Expr::LiteralFloat(0.0)), Op::Sub, Box::new(expr));
        }
        if self.peek() == Some(&Token::Plus) {
            // Unary plus (also covers `+ +x` from macro expansion)
            self.advance();
            return self.parse_primary_expr();
        }
        if self.peek() == Some(&Token::Not) {
            self.advance();
            let expr = self.parse_primary_expr();
            return Expr::FunctionCall("!".into(), vec![expr]);
        }
        // prefix ++ / -- treated as no-op on the value (we just consume them)
        if matches!(self.peek(), Some(Token::PlusPlus) | Some(Token::MinusMinus)) {
            self.advance();
            return self.parse_primary_expr();
        }
        let span = self.peek_span();
        let mut expr = match self.advance() {
            Some(Token::FloatLiteral(f)) => Expr::LiteralFloat(f),
            Some(Token::IntLiteral(i)) => Expr::LiteralInt(i),
            Some(Token::True) => Expr::LiteralBool(true),
            Some(Token::False) => Expr::LiteralBool(false),
            Some(Token::LBrace) => {
                let mut exprs = Vec::new();
                if self.peek() != Some(&Token::RBrace) {
                    loop {
                        exprs.push(self.parse_expr());
                        if self.peek() == Some(&Token::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Token::RBrace, "Expected '}' to end initializer list");
                Expr::InitializerList(exprs)
            }
            Some(Token::Float) => self.parse_constructor(DataType::Float),
            Some(Token::Float2) => self.parse_constructor(DataType::Float2),
            Some(Token::Float3) => self.parse_constructor(DataType::Float3),
            Some(Token::Float4) => self.parse_constructor(DataType::Float4),
            Some(Token::Float4x4) => self.parse_constructor(DataType::Float4x4),
            Some(Token::Float3x3) => self.parse_constructor(DataType::Float3x3),
            Some(Token::Float4x3) => self.parse_constructor(DataType::Float4x3),
            Some(Token::Float3x4) => self.parse_constructor(DataType::Float3x4),
            Some(Token::Float2x2) => self.parse_constructor(DataType::Float2x2),
            Some(Token::Int) => self.parse_constructor(DataType::Int),
            Some(Token::Int2) => self.parse_constructor(DataType::Int2),
            Some(Token::Int3) => self.parse_constructor(DataType::Int3),
            Some(Token::Int4) => self.parse_constructor(DataType::Int4),
            Some(Token::Half) => self.parse_constructor(DataType::Half),
            Some(Token::Half2) => self.parse_constructor(DataType::Half2),
            Some(Token::Half3) => self.parse_constructor(DataType::Half3),
            Some(Token::Half4) => self.parse_constructor(DataType::Half4),
            Some(Token::Bool) => self.parse_constructor(DataType::Bool),
            Some(Token::Identifier(id)) => {
                // Could be a function call, constructor construct, cast, member access, or simple variable
                if self.peek() == Some(&Token::LParen) {
                    self.advance(); // LParen
                    let mut args = Vec::new();
                    if self.peek() != Some(&Token::RParen) {
                        loop {
                            args.push(self.parse_expr());
                            if self.peek() == Some(&Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen, "Expected ')' to close argument list");

                    // Check if identifier is a data type (e.g. float4(1, 2, 3, 4))
                    if let Some(dt) = try_parse_data_type_name(&id) {
                        Expr::Construct(dt, args)
                    } else {
                        Expr::FunctionCall(id, args)
                    }
                } else {
                    Expr::Variable(id)
                }
            }
            Some(Token::LParen) => {
                // Look ahead to detect C-style casts: (type) expr or (const type) expr
                // We need to check if the content of the parens is just a type keyword
                // (possibly preceded by const/static) before calling parse_expr
                let cast_type = {
                    let mut lookahead = self.pos;
                    // Skip const/static
                    while matches!(
                        self.tokens.get(lookahead).map(|(t, _)| t),
                        Some(Token::Const) | Some(Token::Static)
                    ) {
                        lookahead += 1;
                    }
                    // Check if next token is a type keyword
                    let maybe_type = self.tokens.get(lookahead).map(|(t, _)| t);
                    let dt = match maybe_type {
                        Some(Token::Float) => Some(DataType::Float),
                        Some(Token::Float2) => Some(DataType::Float2),
                        Some(Token::Float3) => Some(DataType::Float3),
                        Some(Token::Float4) => Some(DataType::Float4),
                        Some(Token::Float4x4) => Some(DataType::Float4x4),
                        Some(Token::Float3x3) => Some(DataType::Float3x3),
                        Some(Token::Float4x3) => Some(DataType::Float4x3),
                        Some(Token::Float3x4) => Some(DataType::Float3x4),
                        Some(Token::Float2x2) => Some(DataType::Float2x2),
                        Some(Token::Int) => Some(DataType::Int),
                        Some(Token::Int2) => Some(DataType::Int2),
                        Some(Token::Int3) => Some(DataType::Int3),
                        Some(Token::Int4) => Some(DataType::Int4),
                        Some(Token::Half) => Some(DataType::Half),
                        Some(Token::Half2) => Some(DataType::Half2),
                        Some(Token::Half3) => Some(DataType::Half3),
                        Some(Token::Half4) => Some(DataType::Half4),
                        Some(Token::Bool) => Some(DataType::Bool),
                        Some(Token::Identifier(name)) => Some(DataType::UserType(name.clone())),
                        _ => None,
                    };
                    if dt.is_some() {
                        lookahead += 1;
                        // Check if token after type is RParen
                        if matches!(
                            self.tokens.get(lookahead).map(|(t, _)| t),
                            Some(Token::RParen)
                        ) {
                            lookahead += 1;
                            // Check the token after RParen is an expr start
                            let after = self.tokens.get(lookahead).map(|(t, _)| t);
                            let is_expr = matches!(
                                after,
                                Some(Token::Identifier(_))
                                    | Some(Token::FloatLiteral(_))
                                    | Some(Token::IntLiteral(_))
                                    | Some(Token::True)
                                    | Some(Token::False)
                                    | Some(Token::LParen)
                                    | Some(Token::Minus)
                                    | Some(Token::Plus)
                                    | Some(Token::Float)
                                    | Some(Token::Float2)
                                    | Some(Token::Float3)
                                    | Some(Token::Float4)
                                    | Some(Token::Float4x4)
                                    | Some(Token::Half)
                                    | Some(Token::Half2)
                                    | Some(Token::Half3)
                                    | Some(Token::Half4)
                                    | Some(Token::Bool)
                            );
                            if is_expr { dt } else { None }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(dt) = cast_type {
                    // Consume const/static, type keyword, and RParen
                    while matches!(self.peek(), Some(Token::Const) | Some(Token::Static)) {
                        self.advance();
                    }
                    self.advance(); // type keyword
                    self.expect(Token::RParen, "Expected ')' after cast type");
                    let cast_expr = self.parse_primary_expr();
                    Expr::Cast(dt, Box::new(cast_expr))
                } else {
                    let inner = self.parse_expr();
                    self.expect(Token::RParen, "Expected ')' to match '('");
                    inner
                }
            }
            _ => self.error("Expected expression", span, "invalid expression start"),
        };

        // Postfix loop:
        loop {
            if self.peek() == Some(&Token::Dot) {
                self.advance();
                let member_span = self.peek_span();
                let member = match self.advance() {
                    Some(Token::Identifier(m)) => m,
                    _ => self.error(
                        "Expected member name after '.'",
                        member_span,
                        "expected member",
                    ),
                };
                expr = Expr::MemberAccess(Box::new(expr), member);
            } else if self.peek() == Some(&Token::LBracket) {
                self.advance();
                let _index_expr = self.parse_expr();
                self.expect(Token::RBracket, "Expected ']' after array index");
                expr = Expr::MemberAccess(Box::new(expr), "__index".to_string());
            } else if matches!(self.peek(), Some(Token::PlusPlus) | Some(Token::MinusMinus)) {
                // postfix ++ / -- : consume and return expression unchanged (no-op for bytecode)
                self.advance();
            } else {
                break;
            }
        }
        expr
    }

    fn parse_constructor(&mut self, dt: DataType) -> Expr {
        self.expect(Token::LParen, "Expected '(' after type name");
        let mut args = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                args.push(self.parse_expr());
                if self.peek() == Some(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen, "Expected ')' to close argument list");
        Expr::Construct(dt, args)
    }

    fn is_next_expr_start(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::FloatLiteral(_))
                | Some(Token::IntLiteral(_))
                | Some(Token::True)
                | Some(Token::False)
                | Some(Token::Identifier(_))
                | Some(Token::LParen)
                | Some(Token::Minus)
                | Some(Token::Plus)
                | Some(Token::Float)
                | Some(Token::Float2)
                | Some(Token::Float3)
                | Some(Token::Float4)
                | Some(Token::Float4x4)
                | Some(Token::Float3x3)
                | Some(Token::Float4x3)
                | Some(Token::Float3x4)
                | Some(Token::Float2x2)
                | Some(Token::Int)
                | Some(Token::Int2)
                | Some(Token::Int3)
                | Some(Token::Int4)
                | Some(Token::Half)
                | Some(Token::Half2)
                | Some(Token::Half3)
                | Some(Token::Half4)
                | Some(Token::Bool)
        )
    }
}

fn op_precedence(op: &Op) -> u8 {
    match op {
        Op::Assign | Op::AddAssign | Op::SubAssign | Op::MulAssign | Op::DivAssign => 0,
        Op::Or => 1,
        Op::And => 2,
        Op::Equal | Op::NotEqual => 3,
        Op::GreaterThan | Op::LessThan | Op::GreaterThanEqual | Op::LessThanEqual => 4,
        Op::Add | Op::Sub => 5,
        Op::Mul | Op::Div => 6,
    }
}

fn try_parse_data_type_name(name: &str) -> Option<DataType> {
    match name {
        "float" => Some(DataType::Float),
        "float2" => Some(DataType::Float2),
        "float3" => Some(DataType::Float3),
        "float4" => Some(DataType::Float4),
        "float4x4" => Some(DataType::Float4x4),
        "float3x3" => Some(DataType::Float3x3),
        "float4x3" => Some(DataType::Float4x3),
        "float3x4" => Some(DataType::Float3x4),
        "float2x2" => Some(DataType::Float2x2),
        "int" => Some(DataType::Int),
        "int2" => Some(DataType::Int2),
        "int3" => Some(DataType::Int3),
        "int4" => Some(DataType::Int4),
        "half" => Some(DataType::Half),
        "half2" => Some(DataType::Half2),
        "half3" => Some(DataType::Half3),
        "half4" => Some(DataType::Half4),
        "bool" => Some(DataType::Bool),
        _ => None,
    }
}

fn expr_to_datatype(expr: Expr) -> DataType {
    match expr {
        Expr::Variable(name) => {
            if let Some(dt) = try_parse_data_type_name(&name) {
                dt
            } else {
                DataType::UserType(name)
            }
        }
        _ => DataType::Void,
    }
}
