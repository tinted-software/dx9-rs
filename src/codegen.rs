use crate::ast::*;
use std::collections::HashMap;

// D3D9 Opcodes
const OP_NOP: u32 = 0;
const OP_MOV: u32 = 1;
const OP_ADD: u32 = 2;
const OP_SUB: u32 = 3;
const OP_MAD: u32 = 4;
const OP_MUL: u32 = 5;
const OP_RCP: u32 = 6;
const OP_RSQ: u32 = 7;
const OP_DP3: u32 = 8;
const OP_DP4: u32 = 9;
const OP_MIN: u32 = 10;
const OP_MAX: u32 = 11;
const OP_DCL: u32 = 30;
const OP_TEX: u32 = 66; // TEXLD in PS 2.0+
const OP_END: u32 = 0x0000FFFF;

// D3D9 Register Types (D3DSHADER_PARAM_REGISTER_TYPE)
const D3DSPR_TEMP: u32 = 0; // r#
const D3DSPR_INPUT: u32 = 1; // v#
const D3DSPR_CONST: u32 = 2; // c#
#[allow(dead_code)]
const D3DSPR_TEXTURE: u32 = 3; // t# (PS1/2) / a# (VS)
#[allow(dead_code)]
const D3DSPR_RASTOUT: u32 = 4; // oPos, oFog, oPts (VS1/2)
#[allow(dead_code)]
const D3DSPR_ATTROUT: u32 = 5; // oD0, oD1 (VS1/2)
const D3DSPR_OUTPUT: u32 = 6; // o# (VS3 / TEXCRDOUT)
const D3DSPR_COLOROUT: u32 = 8; // oC# (PS2/3 render target)
const D3DSPR_DEPTHOUT: u32 = 9; // oDepth
const D3DSPR_SAMPLER: u32 = 10; // s#

// D3D9 Semantic Usages (D3DDECLUSAGE)
const DECLUSAGE_POSITION: u32 = 0;
const DECLUSAGE_BLENDWEIGHT: u32 = 1;
const DECLUSAGE_BLENDINDICES: u32 = 2;
const DECLUSAGE_NORMAL: u32 = 3;
const DECLUSAGE_PSIZE: u32 = 4;
const DECLUSAGE_TEXCOORD: u32 = 5;
const DECLUSAGE_TANGENT: u32 = 6;
const DECLUSAGE_BINORMAL: u32 = 7;
const DECLUSAGE_COLOR: u32 = 10;
const DECLUSAGE_FOG: u32 = 11;
const DECLUSAGE_DEPTH: u32 = 12;

#[derive(Debug, Clone)]
struct ResolvedRegister {
    reg_type: u32,
    index: u32,
}

pub struct Codegen {
    bytecode: Vec<u32>,
    structs: HashMap<String, StructDef>,
    globals: HashMap<String, VariableDecl>,
    // Mapping from local variables to registers
    local_vars: HashMap<String, ResolvedRegister>,
    // Locals whose fields are bound directly to shader output registers
    output_struct_locals: HashMap<String, String>,
    // Next available temporary register index
    next_temp: u32,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            bytecode: Vec::new(),
            structs: HashMap::new(),
            globals: HashMap::new(),
            local_vars: HashMap::new(),
            output_struct_locals: HashMap::new(),
            next_temp: 0,
        }
    }

    fn alloc_temp(&mut self) -> ResolvedRegister {
        let reg = ResolvedRegister {
            reg_type: D3DSPR_TEMP,
            index: self.next_temp,
        };
        self.next_temp += 1;
        reg
    }

    fn encode_register(reg_type: u32, index: u32, write_mask_or_swizzle: u32) -> u32 {
        let mut token = 0x80000000;
        token |= (reg_type & 0x7) << 28;
        token |= ((reg_type >> 3) & 0x3) << 11;
        token |= index & 0x7FF;
        token |= write_mask_or_swizzle;
        token
    }

    /// Map an HLSL output semantic to the SM3 destination register.
    fn map_output_register(
        &self,
        semantic: &str,
        is_pixel_shader: bool,
        output_reg_idx: &mut u32,
    ) -> ResolvedRegister {
        let sem = semantic.to_ascii_uppercase();
        if is_pixel_shader {
            if sem.starts_with("DEPTH") {
                return ResolvedRegister {
                    reg_type: D3DSPR_DEPTHOUT,
                    index: 0,
                };
            }
            // COLOR / COLOR0 / COLOR1 / SV_TARGET → oC#
            let idx = if let Some(rest) = sem.strip_prefix("COLOR") {
                rest.parse::<u32>().unwrap_or(0)
            } else if let Some(rest) = sem.strip_prefix("SV_TARGET") {
                rest.parse::<u32>().unwrap_or(0)
            } else {
                0
            };
            ResolvedRegister {
                reg_type: D3DSPR_COLOROUT,
                index: idx,
            }
        } else {
            // VS 3.0 uses the generic output file (o#) for every varying.
            let index = *output_reg_idx;
            *output_reg_idx += 1;
            ResolvedRegister {
                reg_type: D3DSPR_OUTPUT,
                index,
            }
        }
    }

    /// Emit `dcl[_semantic] o#` / `dcl oC#` for an SM3 output register.
    fn emit_output_dcl(&mut self, reg: &ResolvedRegister, semantic: &str, is_pixel_shader: bool) {
        let reg_token = Self::encode_register(reg.reg_type, reg.index, 0x000F0000);
        let dcl_usage_token = if is_pixel_shader && reg.reg_type == D3DSPR_COLOROUT {
            // Encode COLOR usage so dxbc-spirv maps oC# → color RT (not POSITION).
            0x80000000 | DECLUSAGE_COLOR | (reg.index << 16)
        } else if is_pixel_shader && reg.reg_type == D3DSPR_DEPTHOUT {
            0x80000000 | DECLUSAGE_DEPTH
        } else {
            let (usage, usage_idx) = parse_semantic(semantic);
            0x80000000 | usage | (usage_idx << 16)
        };

        self.bytecode.push((2 << 24) | OP_DCL);
        self.bytecode.push(dcl_usage_token);
        self.bytecode.push(reg_token);
    }

    pub fn compile(mut self, ast: &ShaderAST, is_pixel_shader: bool) -> Vec<u32> {
        // 1. Gather structs and globals
        for def in &ast.definitions {
            match def {
                Definition::Struct(s) => {
                    self.structs.insert(s.name.clone(), s.clone());
                }
                Definition::Variable(v) => {
                    self.globals.insert(v.name.clone(), v.clone());
                }
                _ => {}
            }
        }

        // 2. Emit version token
        // e.g. vs_3_0 = 0xFFFE0300, ps_3_0 = 0xFFFF0300
        let version_token = if is_pixel_shader {
            0xFFFF0300
        } else {
            0xFFFE0300
        };
        self.bytecode.push(version_token);

        // 3. Find main function
        let main_func = ast.definitions.iter().find_map(|def| match def {
            Definition::Function(f) if f.name == "main" => Some(f),
            _ => None,
        });

        let main_func = match main_func {
            Some(f) => f,
            None => {
                // If there's no main, return empty or helper bytecode
                self.bytecode.push(OP_END);
                return self.bytecode;
            }
        };

        // 4. Map inputs, outputs, and uniforms
        let mut input_reg_idx = 0;
        let mut output_reg_idx = 0;

        // Process function parameters (inputs)
        for param in &main_func.params {
            match &param.data_type {
                DataType::UserType(struct_name) => {
                    if let Some(s_def) = self.structs.get(struct_name).cloned() {
                        for field in &s_def.fields {
                            let semantic = field.semantic.as_deref().unwrap_or("POSITION");
                            let (usage, usage_idx) = parse_semantic(semantic);

                            // Emit DCL token for input register
                            let reg_token =
                                Self::encode_register(D3DSPR_INPUT, input_reg_idx, 0x000F0000);
                            let dcl_usage_token = 0x80000000 | usage | (usage_idx << 16);

                            // DCL instruction token: length 2, opcode DCL
                            self.bytecode.push((2 << 24) | OP_DCL);
                            self.bytecode.push(dcl_usage_token);
                            self.bytecode.push(reg_token);

                            // Store register mapping for param.field
                            let key = format!("{}.{}", param.name, field.name);
                            self.local_vars.insert(
                                key,
                                ResolvedRegister {
                                    reg_type: D3DSPR_INPUT,
                                    index: input_reg_idx,
                                },
                            );
                            input_reg_idx += 1;
                        }
                    }
                }
                _ => {
                    // Simple parameters
                    let semantic = param.semantic.as_deref().unwrap_or("POSITION");
                    let (usage, usage_idx) = parse_semantic(semantic);

                    let reg_token = Self::encode_register(D3DSPR_INPUT, input_reg_idx, 0x000F0000);
                    let dcl_usage_token = 0x80000000 | usage | (usage_idx << 16);

                    self.bytecode.push((2 << 24) | OP_DCL);
                    self.bytecode.push(dcl_usage_token);
                    self.bytecode.push(reg_token);

                    self.local_vars.insert(
                        param.name.clone(),
                        ResolvedRegister {
                            reg_type: D3DSPR_INPUT,
                            index: input_reg_idx,
                        },
                    );
                    input_reg_idx += 1;
                }
            }
        }

        // Map global variables / uniforms
        for (name, var) in &self.globals {
            if let Some(reg) = &var.register {
                match reg {
                    RegisterType::ConstantFloat(idx) => {
                        self.local_vars.insert(
                            name.clone(),
                            ResolvedRegister {
                                reg_type: D3DSPR_CONST,
                                index: *idx as u32,
                            },
                        );
                    }
                    RegisterType::Sampler(idx) => {
                        // Sampler declaration instruction
                        let reg_token =
                            Self::encode_register(D3DSPR_SAMPLER, *idx as u32, 0x000F0000);
                        self.bytecode.push((2 << 24) | OP_DCL);
                        self.bytecode.push(0x80000000); // Default sampler type
                        self.bytecode.push(reg_token);

                        self.local_vars.insert(
                            name.clone(),
                            ResolvedRegister {
                                reg_type: D3DSPR_SAMPLER,
                                index: *idx as u32,
                            },
                        );
                    }
                }
            }
        }

        // Setup outputs structure mapping.
        // SM3 pixel shaders write COLOR/DEPTH to oC# / oDepth and must dcl them.
        // SM3 vertex shaders write all varyings to generic o# registers with dcl+semantic.
        let mut output_mapping = HashMap::new();
        match &main_func.return_type {
            DataType::UserType(struct_name) => {
                if let Some(s_def) = self.structs.get(struct_name).cloned() {
                    for field in &s_def.fields {
                        let semantic = field.semantic.as_deref().unwrap_or("POSITION");
                        let out_reg = self.map_output_register(
                            semantic,
                            is_pixel_shader,
                            &mut output_reg_idx,
                        );
                        self.emit_output_dcl(&out_reg, semantic, is_pixel_shader);
                        output_mapping.insert(field.name.clone(), out_reg);
                    }
                }
            }
            _ => {
                let semantic = main_func
                    .return_semantic
                    .as_deref()
                    .unwrap_or(if is_pixel_shader { "COLOR" } else { "POSITION" });
                let out_reg =
                    self.map_output_register(semantic, is_pixel_shader, &mut output_reg_idx);
                self.emit_output_dcl(&out_reg, semantic, is_pixel_shader);
                output_mapping.insert("return".to_string(), out_reg);
            }
        }

        // 5. Compile body
        for stmt in &main_func.body {
            self.compile_stmt(stmt, &output_mapping);
        }

        // 6. Emit end token
        self.bytecode.push(OP_END);

        self.bytecode
    }

    fn compile_stmt(&mut self, stmt: &Stmt, output_mapping: &HashMap<String, ResolvedRegister>) {
        match stmt {
            Stmt::VariableDecl(v) => {
                // `VS_OUT o;` — bind o.field to the declared SM3 output registers so
                // assignments like `o.pos = ...` write o# / oC# directly.
                if let DataType::UserType(struct_name) = &v.data_type {
                    if let Some(s_def) = self.structs.get(struct_name).cloned() {
                        let mut bound = false;
                        for field in &s_def.fields {
                            if let Some(out_reg) = output_mapping.get(&field.name) {
                                let key = format!("{}.{}", v.name, field.name);
                                self.local_vars.insert(key, out_reg.clone());
                                bound = true;
                            }
                        }
                        if bound {
                            self.output_struct_locals
                                .insert(v.name.clone(), struct_name.clone());
                            return;
                        }
                    }
                }

                let temp = self.alloc_temp();
                self.local_vars.insert(v.name.clone(), temp.clone());
                if let Some(init) = &v.initializer {
                    let src = self.compile_expr(init);
                    self.emit_instruction(OP_MOV, &temp, &[src]);
                }
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr);
            }
            Stmt::Return(expr_opt) => {
                if let Some(expr) = expr_opt {
                    // Returning an output struct local: fields were already written to o#/oC#.
                    if let Expr::Variable(name) = expr {
                        if self.output_struct_locals.contains_key(name) {
                            return;
                        }
                    }
                    let src = self.compile_expr(expr);
                    if let Some(out_reg) = output_mapping.get("return") {
                        self.emit_instruction(OP_MOV, out_reg, &[src]);
                    }
                }
            }
            Stmt::Block(stmts) => {
                for s in stmts {
                    self.compile_stmt(s, output_mapping);
                }
            }
            Stmt::If(cond, then_branch, else_branch) => {
                // Compile standard IF statement
                self.compile_expr(cond);
                self.compile_stmt(then_branch, output_mapping);
                if let Some(eb) = else_branch {
                    self.compile_stmt(eb, output_mapping);
                }
            }
            Stmt::For(init, cond, post, body) => {
                self.compile_stmt(init, output_mapping);
                if let Some(c) = cond {
                    self.compile_expr(c);
                }
                self.compile_stmt(body, output_mapping);
                if let Some(p) = post {
                    self.compile_expr(p);
                }
            }
            Stmt::While(cond, body) => {
                self.compile_expr(cond);
                self.compile_stmt(body, output_mapping);
            }
            Stmt::Break => {
                // no-op for now
            }
        }
    }

    fn compile_expr(&mut self, expr: &Expr) -> ResolvedRegister {
        match expr {
            Expr::LiteralFloat(_) | Expr::LiteralInt(_) | Expr::LiteralBool(_) => {
                // Allocate a constant/temp to hold literals, or return a fake temp register
                let temp = self.alloc_temp();
                // D3D9 literal definitions are usually done via DEF instruction, but for simplicty,
                // we can map to temp.
                temp
            }
            Expr::Variable(name) => {
                if let Some(reg) = self.local_vars.get(name) {
                    reg.clone()
                } else {
                    // Fallback to temp
                    self.alloc_temp()
                }
            }
            Expr::MemberAccess(base, member) => {
                if let Expr::Variable(base_name) = &**base {
                    let key = format!("{}.{}", base_name, member);
                    if let Some(reg) = self.local_vars.get(&key) {
                        return reg.clone();
                    }
                }
                self.compile_expr(base)
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                if matches!(
                    op,
                    Op::Assign | Op::AddAssign | Op::SubAssign | Op::MulAssign | Op::DivAssign
                ) {
                    let r_reg = self.compile_expr(rhs);
                    let l_reg = self.compile_expr(lhs);
                    self.emit_instruction(OP_MOV, &l_reg, &[r_reg]);
                    l_reg
                } else {
                    let l_reg = self.compile_expr(lhs);
                    let r_reg = self.compile_expr(rhs);
                    let dest = self.alloc_temp();
                    let opcode = match op {
                        Op::Add => OP_ADD,
                        Op::Sub => OP_SUB,
                        Op::Mul => OP_MUL,
                        Op::Div => OP_RCP, // RCP is reciprocal, Div can be implemented via RCP + MUL
                        Op::Assign => unreachable!(),
                        _ => OP_ADD,
                    };
                    if opcode == OP_RCP {
                        let rcp_temp = self.alloc_temp();
                        self.emit_instruction(OP_RCP, &rcp_temp, &[r_reg]);
                        self.emit_instruction(OP_MUL, &dest, &[l_reg, rcp_temp]);
                    } else {
                        self.emit_instruction(opcode, &dest, &[l_reg, r_reg]);
                    }
                    dest
                }
            }
            Expr::Construct(_, args) => {
                // Construct a vector. Allocate temp and move/evaluate arguments.
                let dest = self.alloc_temp();
                if !args.is_empty() {
                    let src = self.compile_expr(&args[0]);
                    self.emit_instruction(OP_MOV, &dest, &[src]);
                }
                dest
            }
            Expr::FunctionCall(name, args) => {
                let dest = self.alloc_temp();
                match name.as_str() {
                    "mul" if args.len() == 2 => {
                        let a = self.compile_expr(&args[0]);
                        let b = self.compile_expr(&args[1]);
                        // Matrix multiply can be implemented as series of DP4/DP3 instructions
                        self.emit_instruction(OP_DP4, &dest, &[a, b]);
                    }
                    "tex2D" if args.len() == 2 => {
                        let sampler = self.compile_expr(&args[0]);
                        let uv = self.compile_expr(&args[1]);
                        // texld dest, uv, sampler
                        self.emit_instruction(OP_TEX, &dest, &[uv, sampler]);
                    }
                    _ => {
                        if !args.is_empty() {
                            let src = self.compile_expr(&args[0]);
                            self.emit_instruction(OP_MOV, &dest, &[src]);
                        }
                    }
                }
                dest
            }
            Expr::Cast(_, base) => self.compile_expr(base),
            Expr::Ternary(cond, then_expr, else_expr) => {
                self.compile_expr(cond);
                let t_reg = self.compile_expr(then_expr);
                self.compile_expr(else_expr);
                t_reg
            }
            Expr::InitializerList(exprs) => {
                for expr in exprs {
                    self.compile_expr(expr);
                }
                self.alloc_temp()
            }
        }
    }

    fn emit_instruction(
        &mut self,
        opcode: u32,
        dest: &ResolvedRegister,
        sources: &[ResolvedRegister],
    ) {
        let size = 1 + sources.len() as u32; // dest + sources
        let inst_token = (size << 24) | opcode;
        self.bytecode.push(inst_token);

        // Dest register token (all channels write mask 0x000F0000 by default)
        let dest_token = Self::encode_register(dest.reg_type, dest.index, 0x000F0000);
        self.bytecode.push(dest_token);

        // Source register tokens (XYZW identity swizzle 0x00E40000 by default)
        for src in sources {
            let src_token = Self::encode_register(src.reg_type, src.index, 0x00E40000);
            self.bytecode.push(src_token);
        }
    }
}

fn parse_semantic(sem: &str) -> (u32, u32) {
    let sem_upper = sem.to_ascii_uppercase();
    if sem_upper.starts_with("POSITION") {
        let idx = sem_upper
            .strip_prefix("POSITION")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_POSITION, idx)
    } else if sem_upper.starts_with("NORMAL") {
        let idx = sem_upper
            .strip_prefix("NORMAL")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_NORMAL, idx)
    } else if sem_upper.starts_with("BLENDWEIGHT") {
        let idx = sem_upper
            .strip_prefix("BLENDWEIGHT")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_BLENDWEIGHT, idx)
    } else if sem_upper.starts_with("BLENDINDICES") {
        let idx = sem_upper
            .strip_prefix("BLENDINDICES")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_BLENDINDICES, idx)
    } else if sem_upper.starts_with("TEXCOORD") {
        let idx = sem_upper
            .strip_prefix("TEXCOORD")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_TEXCOORD, idx)
    } else if sem_upper.starts_with("TANGENT") {
        let idx = sem_upper
            .strip_prefix("TANGENT")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_TANGENT, idx)
    } else if sem_upper.starts_with("BINORMAL") {
        let idx = sem_upper
            .strip_prefix("BINORMAL")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_BINORMAL, idx)
    } else if sem_upper.starts_with("COLOR") {
        let idx = sem_upper
            .strip_prefix("COLOR")
            .unwrap()
            .parse::<u32>()
            .unwrap_or(0);
        (DECLUSAGE_COLOR, idx)
    } else if sem_upper.starts_with("PSIZE") || sem_upper.starts_with("POINTSIZE") {
        (DECLUSAGE_PSIZE, 0)
    } else if sem_upper.starts_with("FOG") {
        (DECLUSAGE_FOG, 0)
    } else if sem_upper.starts_with("DEPTH") {
        (DECLUSAGE_DEPTH, 0)
    } else {
        (DECLUSAGE_POSITION, 0)
    }
}
