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
const OP_EXP: u32 = 14; // exp2 approx (D3DSIO_EXP)
const OP_LOG: u32 = 15; // log2 approx (D3DSIO_LOG)
const OP_LRP: u32 = 18; // lerp
const OP_FRC: u32 = 19;
const OP_M4X4: u32 = 20;
const OP_M4X3: u32 = 21;
const OP_M3X3: u32 = 23;
const OP_DCL: u32 = 31; // D3DSIO_DCL (30 is D3DSIO_LABEL)
const OP_POW: u32 = 32; // dest = src0 ^ src1
const OP_ABS: u32 = 35;
const OP_NRM: u32 = 36;
const OP_TEX: u32 = 66; // TEXLD in PS 2.0+
const OP_DEF: u32 = 81; // DEF c#, f,f,f,f
const OP_TEXLDL: u32 = 95; // TEXLDL (explicit LOD in .w)
const OP_END: u32 = 0x0000FFFF;
const WRITEMASK_X: u32 = 0x0001_0000;
const WRITEMASK_Y: u32 = 0x0002_0000;
const WRITEMASK_Z: u32 = 0x0004_0000;
const WRITEMASK_W: u32 = 0x0008_0000;
const WRITEMASK_XYZ: u32 = WRITEMASK_X | WRITEMASK_Y | WRITEMASK_Z;
const WRITEMASK_XYZW: u32 = WRITEMASK_XYZ | WRITEMASK_W;
const SWIZZLE_XYZW: u32 = 0x00E4_0000; // identity swizzle
const SWIZZLE_XXXX: u32 = 0x0000_0000;
const SWIZZLE_YYYY: u32 = 0x0055_0000;
const SWIZZLE_ZZZZ: u32 = 0x00AA_0000;
const SWIZZLE_WWWW: u32 = 0x00FF_0000;
const TEXLD_PROJECT: u32 = 0x0001_0000; // D3DSI_TEXLD_PROJECT
const TEXLD_BIAS: u32 = 0x0002_0000; // D3DSI_TEXLD_BIAS
// D3DSTT_* in bits 27-30 of the dcl texture-type token
const D3DSTT_2D: u32 = 2;
const D3DSTT_CUBE: u32 = 3;
const D3DSTT_VOLUME: u32 = 4;

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

/// Map HLSL swizzle/component names to a D3D9 source swizzle token.
/// Scalar selectors (.r/.a/…) replicate into all four channels so `float`
/// temps that conventionally live in `.x` still see the right value.
fn swizzle_from_member(member: &str) -> Option<u32> {
    match member {
        "x" | "r" | "s" => Some(SWIZZLE_XXXX),
        "y" | "g" | "t" => Some(SWIZZLE_YYYY),
        "z" | "b" | "p" => Some(SWIZZLE_ZZZZ),
        "w" | "a" | "q" => Some(SWIZZLE_WWWW),
        "xy" | "rg" | "st" => Some(0x0044_0000), // XYXY
        "zw" | "ba" | "pq" => Some(0x00EE_0000), // ZWZW
        "xyz" | "rgb" | "stp" => Some(SWIZZLE_XYZW),
        "xyzw" | "rgba" | "stpq" => Some(SWIZZLE_XYZW),
        _ => None,
    }
}

fn writemask_from_member(member: &str) -> Option<u32> {
    match member {
        "x" | "r" | "s" => Some(WRITEMASK_X),
        "y" | "g" | "t" => Some(WRITEMASK_Y),
        "z" | "b" | "p" => Some(WRITEMASK_Z),
        "w" | "a" | "q" => Some(WRITEMASK_W),
        "xy" | "rg" | "st" => Some(WRITEMASK_X | WRITEMASK_Y),
        "zw" | "ba" | "pq" => Some(WRITEMASK_Z | WRITEMASK_W),
        "xyz" | "rgb" | "stp" => Some(WRITEMASK_XYZ),
        "xyzw" | "rgba" | "stpq" => Some(WRITEMASK_XYZW),
        _ => None,
    }
}

/// Source swizzle when writing a (possibly scalar) value into a masked dest.
/// D3D feeds the Nth written channel from the Nth swizzle slot; for a lone
/// `.w` write only the W slot matters, so XXXX puts src.x into dest.w.
fn src_swizzle_for_writemask(mask: u32) -> u32 {
    if mask == WRITEMASK_X || mask == WRITEMASK_Y || mask == WRITEMASK_Z || mask == WRITEMASK_W {
        SWIZZLE_XXXX
    } else {
        SWIZZLE_XYZW
    }
}

pub struct Codegen {
    bytecode: Vec<u32>,
    structs: HashMap<String, StructDef>,
    globals: HashMap<String, VariableDecl>,
    // Mapping from local variables to registers
    local_vars: HashMap<String, ResolvedRegister>,
    // Locals whose fields are bound directly to shader output registers
    output_struct_locals: HashMap<String, String>,
    // Compile-time bools from `static const bool x = ...` (and similar)
    const_bools: HashMap<String, bool>,
    // Next available temporary register index
    next_temp: u32,
    // High constant slots reserved for DEF immediates (avoid clobbering engine c0..)
    next_def_const: u32,
    // Matrix-typed constant registers (base index → rows: 4=float4x4, 3=float4x3/float3x3)
    matrix_consts: HashMap<u32, u32>,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            bytecode: Vec::new(),
            structs: HashMap::new(),
            globals: HashMap::new(),
            local_vars: HashMap::new(),
            output_struct_locals: HashMap::new(),
            const_bools: HashMap::new(),
            next_temp: 0,
            // Set properly in compile() from is_pixel_shader — PS SM3 max is c223.
            next_def_const: 240,
            matrix_consts: HashMap::new(),
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

    /// Resolve the register behind an lvalue (no swizzle materialization).
    fn resolve_lvalue_register(&mut self, expr: &Expr) -> Option<ResolvedRegister> {
        match expr {
            Expr::Variable(name) => self.local_vars.get(name).cloned(),
            Expr::MemberAccess(base, member) => {
                if let Expr::Variable(base_name) = &**base {
                    let key = format!("{}.{}", base_name, member);
                    if let Some(reg) = self.local_vars.get(&key) {
                        return Some(reg.clone());
                    }
                }
                // Nested swizzle lvalue like `o.color.a` — strip the mask later.
                self.resolve_lvalue_register(base)
            }
            _ => None,
        }
    }

    /// `(register, writemask)` for assignments to `foo.a` / `bar.rgb` / etc.
    fn resolve_masked_lvalue(&mut self, expr: &Expr) -> Option<(ResolvedRegister, u32)> {
        match expr {
            Expr::MemberAccess(base, member) => {
                if let Some(mask) = writemask_from_member(member) {
                    let reg = self.resolve_lvalue_register(base)?;
                    return Some((reg, mask));
                }
                // `o.color` style struct field — full register write.
                if let Expr::Variable(base_name) = &**base {
                    let key = format!("{}.{}", base_name, member);
                    if let Some(reg) = self.local_vars.get(&key) {
                        return Some((reg.clone(), WRITEMASK_XYZW));
                    }
                }
                None
            }
            Expr::Variable(name) => self
                .local_vars
                .get(name)
                .cloned()
                .map(|r| (r, WRITEMASK_XYZW)),
            _ => None,
        }
    }

    fn emit_assign_op(&mut self, op: Op, lhs: &Expr, rhs: &Expr) -> ResolvedRegister {
        let r_reg = self.compile_expr(rhs);
        if let Some((l_reg, mask)) = self.resolve_masked_lvalue(lhs) {
            let src_swz = src_swizzle_for_writemask(mask);
            match op {
                Op::Assign => {
                    self.emit_mov_masked(&l_reg, &r_reg, mask, src_swz);
                }
                Op::AddAssign | Op::SubAssign | Op::MulAssign | Op::DivAssign => {
                    let opcode = match op {
                        Op::AddAssign => OP_ADD,
                        Op::SubAssign => OP_SUB,
                        Op::MulAssign => OP_MUL,
                        Op::DivAssign => OP_RCP, // handled below
                        _ => unreachable!(),
                    };
                    if opcode == OP_RCP {
                        // l.mask /= r  →  l.mask = l * rcp(r)
                        let rcp_temp = self.alloc_temp();
                        self.emit_instruction(OP_RCP, &rcp_temp, &[r_reg]);
                        self.bytecode.push((3 << 24) | OP_MUL);
                        self.bytecode.push(Self::encode_register(
                            l_reg.reg_type,
                            l_reg.index,
                            mask,
                        ));
                        self.bytecode.push(Self::encode_register(
                            l_reg.reg_type,
                            l_reg.index,
                            SWIZZLE_XYZW,
                        ));
                        self.bytecode.push(Self::encode_register(
                            rcp_temp.reg_type,
                            rcp_temp.index,
                            src_swz,
                        ));
                    } else {
                        self.bytecode.push((3 << 24) | opcode);
                        self.bytecode.push(Self::encode_register(
                            l_reg.reg_type,
                            l_reg.index,
                            mask,
                        ));
                        self.bytecode.push(Self::encode_register(
                            l_reg.reg_type,
                            l_reg.index,
                            SWIZZLE_XYZW,
                        ));
                        self.bytecode.push(Self::encode_register(
                            r_reg.reg_type,
                            r_reg.index,
                            src_swz,
                        ));
                    }
                }
                _ => {}
            }
            return l_reg;
        }
        // Fallback: whole-register ops (no known lvalue).
        let l_reg = self.compile_expr(lhs);
        if r_reg.reg_type != D3DSPR_SAMPLER && l_reg.reg_type != D3DSPR_SAMPLER {
            match op {
                Op::Assign => self.emit_instruction(OP_MOV, &l_reg, &[r_reg]),
                Op::AddAssign => self.emit_instruction(OP_ADD, &l_reg, &[l_reg.clone(), r_reg]),
                Op::SubAssign => self.emit_instruction(OP_SUB, &l_reg, &[l_reg.clone(), r_reg]),
                Op::MulAssign => self.emit_instruction(OP_MUL, &l_reg, &[l_reg.clone(), r_reg]),
                Op::DivAssign => {
                    let rcp_temp = self.alloc_temp();
                    self.emit_instruction(OP_RCP, &rcp_temp, &[r_reg]);
                    self.emit_instruction(OP_MUL, &l_reg, &[l_reg.clone(), rcp_temp]);
                }
                _ => {}
            }
        }
        l_reg
    }

    fn encode_register(reg_type: u32, index: u32, write_mask_or_swizzle: u32) -> u32 {
        let mut token = 0x80000000;
        token |= (reg_type & 0x7) << 28;
        token |= ((reg_type >> 3) & 0x3) << 11;
        token |= index & 0x7FF;
        token |= write_mask_or_swizzle;
        token
    }

    /// Resolve a compile-time bool (static const, literals, simple logic).
    /// Used so `if (g_bVertexColor)` doesn't emit both branches — that was
    /// overwriting VGUI vertex color with DoLighting (alpha 0 → invisible).
    fn eval_const_bool(&self, expr: &Expr) -> Option<bool> {
        match expr {
            Expr::LiteralBool(v) => Some(*v),
            Expr::LiteralInt(v) => Some(*v != 0),
            Expr::LiteralFloat(v) => Some(*v != 0.0),
            Expr::Variable(name) => self.const_bools.get(name).copied(),
            Expr::BinaryOp(lhs, Op::And, rhs) => {
                Some(self.eval_const_bool(lhs)? && self.eval_const_bool(rhs)?)
            }
            Expr::BinaryOp(lhs, Op::Or, rhs) => {
                Some(self.eval_const_bool(lhs)? || self.eval_const_bool(rhs)?)
            }
            Expr::Ternary(cond, t, e) => {
                if self.eval_const_bool(cond)? {
                    self.eval_const_bool(t)
                } else {
                    self.eval_const_bool(e)
                }
            }
            Expr::Cast(_, inner) => self.eval_const_bool(inner),
            // Unary `!` parsed as FunctionCall("!", [expr]) — see lexer Token::Not.
            Expr::FunctionCall(name, args) if name == "!" && args.len() == 1 => {
                self.eval_const_bool(&args[0]).map(|v| !v)
            }
            _ => None,
        }
    }

    fn record_const_from_decl(&mut self, v: &VariableDecl) {
        if let Some(init) = &v.initializer {
            if let Some(b) = self.eval_const_bool(init) {
                self.const_bools.insert(v.name.clone(), b);
            }
        }
    }

    /// Emit `def cN, x,y,z,w` and return that constant register.
    fn def_const(&mut self, x: f32, y: f32, z: f32, w: f32) -> ResolvedRegister {
        let idx = self.next_def_const;
        self.next_def_const += 1;
        let dest = ResolvedRegister {
            reg_type: D3DSPR_CONST,
            index: idx,
        };
        // DEF length = 5 (dest + 4 float immediates)
        self.bytecode.push((5 << 24) | OP_DEF);
        self.bytecode
            .push(Self::encode_register(D3DSPR_CONST, idx, WRITEMASK_XYZW));
        self.bytecode.push(x.to_bits());
        self.bytecode.push(y.to_bits());
        self.bytecode.push(z.to_bits());
        self.bytecode.push(w.to_bits());
        dest
    }

    /// If every arg is a numeric literal, emit one `def` instead of N defs + movs.
    fn try_def_literal_construct(&mut self, args: &[Expr]) -> Option<ResolvedRegister> {
        if args.is_empty() || args.len() > 4 {
            return None;
        }
        let mut comps = [0.0f32; 4];
        for (i, arg) in args.iter().enumerate() {
            comps[i] = match arg {
                Expr::LiteralFloat(v) => *v,
                Expr::LiteralInt(v) => *v as f32,
                Expr::LiteralBool(v) => {
                    if *v {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => return None,
            };
        }
        // Splat last component into unused channels (matches HLSL scalar→vector feel
        // for partial vectors; float4(1,1,1,1) fills all four explicitly).
        if args.len() < 4 {
            let last = comps[args.len() - 1];
            for c in &mut comps[args.len()..] {
                *c = last;
            }
        }
        Some(self.def_const(comps[0], comps[1], comps[2], comps[3]))
    }

    fn emit_mov_masked(
        &mut self,
        dest: &ResolvedRegister,
        src: &ResolvedRegister,
        dest_mask: u32,
        src_swizzle: u32,
    ) {
        self.bytecode.push((2 << 24) | OP_MOV);
        self.bytecode
            .push(Self::encode_register(dest.reg_type, dest.index, dest_mask));
        self.bytecode
            .push(Self::encode_register(src.reg_type, src.index, src_swizzle));
    }

    fn is_matrix_const(&self, reg: &ResolvedRegister) -> Option<u32> {
        if reg.reg_type == D3DSPR_CONST {
            self.matrix_consts.get(&reg.index).copied()
        } else {
            None
        }
    }

    /// Rows for a matrix-typed expression (including casts like `(float2x4)cTex`).
    fn matrix_rows_of_expr(&self, expr: &Expr) -> Option<u32> {
        match expr {
            Expr::Cast(dt, inner) => match dt {
                DataType::Float4x4 => Some(4),
                DataType::Float4x3 | DataType::Float3x3 | DataType::Float3x4 => Some(3),
                DataType::Float2x2 => Some(2),
                DataType::UserType(n)
                    if n.eq_ignore_ascii_case("float2x4") || n.eq_ignore_ascii_case("float2x3") =>
                {
                    Some(2)
                }
                DataType::UserType(n) if n.eq_ignore_ascii_case("float3x4") => Some(3),
                // Cast of unknown type: peek at inner register matrix map
                _ => self.matrix_rows_of_expr(inner),
            },
            Expr::Variable(name) => {
                if let Some(reg) = self.local_vars.get(name) {
                    if let Some(rows) = self.is_matrix_const(reg) {
                        return Some(rows);
                    }
                }
                if let Some(g) = self.globals.get(name) {
                    return match g.data_type {
                        DataType::Float4x4 => Some(4),
                        DataType::Float4x3 | DataType::Float3x3 | DataType::Float3x4 => Some(3),
                        DataType::Float2x2 => Some(2),
                        _ => None,
                    };
                }
                None
            }
            Expr::MemberAccess(base, member) if member == "__index" => {
                self.matrix_rows_of_expr(base)
            }
            _ => None,
        }
    }

    fn emit_dp4_masked(
        &mut self,
        dest: &ResolvedRegister,
        a: &ResolvedRegister,
        b: &ResolvedRegister,
        dest_mask: u32,
    ) {
        self.bytecode.push((3 << 24) | OP_DP4);
        self.bytecode
            .push(Self::encode_register(dest.reg_type, dest.index, dest_mask));
        self.bytecode
            .push(Self::encode_register(a.reg_type, a.index, SWIZZLE_XYZW));
        self.bytecode
            .push(Self::encode_register(b.reg_type, b.index, SWIZZLE_XYZW));
    }

    /// mul(v, floatNxM) via N DP4s into dest.xyzw (texture transforms, etc.).
    fn emit_row_dots(
        &mut self,
        dest: &ResolvedRegister,
        vec: &ResolvedRegister,
        matrix_base: &ResolvedRegister,
        rows: u32,
    ) {
        let masks = [WRITEMASK_X, WRITEMASK_Y, WRITEMASK_Z, WRITEMASK_W];
        for i in 0..rows.min(4) {
            let row = ResolvedRegister {
                reg_type: matrix_base.reg_type,
                index: matrix_base.index + i,
            };
            self.emit_dp4_masked(dest, vec, &row, masks[i as usize]);
        }
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
        // DEF immediates must stay inside the stage's float-const limit:
        //   VS SM3: c0–c255 (MaxFloatConstantsVS = 256)
        //   PS SM3: c0–c223 (MaxSM3FloatConstantsPS = 224)
        // Engine shader-specific VS consts go through ~c224; shared PS consts
        // are low (≤~c31). Use the high end of each legal range.
        self.next_def_const = if is_pixel_shader { 200 } else { 240 };

        // 1. Gather structs and globals
        for def in &ast.definitions {
            match def {
                Definition::Struct(s) => {
                    self.structs.insert(s.name.clone(), s.clone());
                }
                Definition::Variable(v) => {
                    self.record_const_from_decl(v);
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
                        let idx = *idx as u32;
                        self.local_vars.insert(
                            name.clone(),
                            ResolvedRegister {
                                reg_type: D3DSPR_CONST,
                                index: idx,
                            },
                        );
                        let rows = match var.data_type {
                            DataType::Float4x4 => Some(4u32),
                            DataType::Float4x3 | DataType::Float3x3 | DataType::Float3x4 => {
                                Some(3u32)
                            }
                            DataType::Float2x2 => Some(2u32),
                            _ => None,
                        };
                        if let Some(r) = rows {
                            self.matrix_consts.insert(idx, r);
                        }
                    }
                    RegisterType::Sampler(idx) => {
                        // Sampler declaration: dcl_2d / dcl_cube / dcl_volume s#
                        let reg_token =
                            Self::encode_register(D3DSPR_SAMPLER, *idx as u32, 0x000F0000);
                        let tex_ty = match var.data_type {
                            DataType::SamplerCUBE => D3DSTT_CUBE,
                            DataType::Sampler3D => D3DSTT_VOLUME,
                            DataType::UserType(ref n) if n.eq_ignore_ascii_case("samplercube") => {
                                D3DSTT_CUBE
                            }
                            DataType::UserType(ref n) if n.eq_ignore_ascii_case("sampler3d") => {
                                D3DSTT_VOLUME
                            }
                            _ => D3DSTT_2D,
                        };
                        let dcl_texture_type = 0x80000000 | (tex_ty << 27);
                        self.bytecode.push((2 << 24) | OP_DCL);
                        self.bytecode.push(dcl_texture_type);
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
                self.record_const_from_decl(v);
                // `VS_OUT o;` — bind o.field to temporaries. Flushing to real
                // o# / oC# registers happens on `return o` so mid-shader reads
                // of `o.field` stay legal (SM3 outputs are write-only).
                if let DataType::UserType(struct_name) = &v.data_type {
                    if let Some(s_def) = self.structs.get(struct_name).cloned() {
                        let mut bound = false;
                        for field in &s_def.fields {
                            if output_mapping.contains_key(&field.name) {
                                let key = format!("{}.{}", v.name, field.name);
                                let temp = self.alloc_temp();
                                self.local_vars.insert(key, temp);
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
                    // Returning an output struct local: copy field temps → o#/oC#.
                    if let Expr::Variable(name) = expr {
                        if let Some(struct_name) = self.output_struct_locals.get(name).cloned() {
                            if let Some(s_def) = self.structs.get(&struct_name).cloned() {
                                for field in &s_def.fields {
                                    let key = format!("{}.{}", name, field.name);
                                    if let (Some(src), Some(dst)) = (
                                        self.local_vars.get(&key).cloned(),
                                        output_mapping.get(&field.name),
                                    ) {
                                        self.emit_instruction(OP_MOV, dst, &[src]);
                                    }
                                }
                            }
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
                // Fold compile-time conditions so static combo branches don't
                // both execute (last write wins and corrupts outputs).
                match self.eval_const_bool(cond) {
                    Some(true) => self.compile_stmt(then_branch, output_mapping),
                    Some(false) => {
                        if let Some(eb) = else_branch {
                            self.compile_stmt(eb, output_mapping);
                        }
                    }
                    None => {
                        // No real branching yet — skip rather than emit both
                        // (dual-write) or then-only (wrong for `if (!flag)`).
                    }
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
            Expr::LiteralFloat(v) => self.def_const(*v, *v, *v, *v),
            Expr::LiteralInt(v) => {
                let f = *v as f32;
                self.def_const(f, f, f, f)
            }
            Expr::LiteralBool(v) => {
                let f = if *v { 1.0 } else { 0.0 };
                self.def_const(f, f, f, f)
            }
            Expr::Variable(name) => {
                if let Some(reg) = self.local_vars.get(name) {
                    reg.clone()
                } else {
                    self.alloc_temp()
                }
            }
            Expr::MemberAccess(base, member) => {
                if member == "__index" {
                    return self.compile_expr(base);
                }
                // Struct field binding (e.g. o.color → dedicated temp).
                if let Expr::Variable(base_name) = &**base {
                    let key = format!("{}.{}", base_name, member);
                    if let Some(reg) = self.local_vars.get(&key) {
                        return reg.clone();
                    }
                }
                let reg = self.compile_expr(base);
                // Materialize swizzles into a temp. Without this, `baseColor.a`
                // was ignored and `alpha *= baseColor.a` became `alpha *= baseColor.r`
                // — white glyph RGB made every covered texel fully opaque (solid
                // rectangle characters).
                if let Some(swz) = swizzle_from_member(member) {
                    let dest = self.alloc_temp();
                    self.emit_mov_masked(&dest, &reg, WRITEMASK_XYZW, swz);
                    return dest;
                }
                reg
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                if matches!(
                    op,
                    Op::Assign | Op::AddAssign | Op::SubAssign | Op::MulAssign | Op::DivAssign
                ) {
                    return self.emit_assign_op(op.clone(), lhs, rhs);
                }
                let l_reg = self.compile_expr(lhs);
                let r_reg = self.compile_expr(rhs);
                if l_reg.reg_type == D3DSPR_SAMPLER || r_reg.reg_type == D3DSPR_SAMPLER {
                    return self.alloc_temp();
                }
                let dest = self.alloc_temp();
                let opcode = match op {
                    Op::Add => OP_ADD,
                    Op::Sub => OP_SUB,
                    Op::Mul => OP_MUL,
                    Op::Div => OP_RCP,
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
            Expr::Construct(_, args) => {
                if let Some(lit) = self.try_def_literal_construct(args) {
                    return lit;
                }
                let dest = self.alloc_temp();
                if args.is_empty() {
                    return dest;
                }
                if args.len() == 1 {
                    let src = self.compile_expr(&args[0]);
                    if src.reg_type != D3DSPR_SAMPLER {
                        self.emit_instruction(OP_MOV, &dest, &[src]);
                    }
                    return dest;
                }
                // float4(vec3, scalar) — common for clip-space transforms
                if args.len() == 2 {
                    let a = self.compile_expr(&args[0]);
                    let b = self.compile_expr(&args[1]);
                    if a.reg_type != D3DSPR_SAMPLER {
                        self.emit_mov_masked(&dest, &a, WRITEMASK_XYZ, SWIZZLE_XYZW);
                    }
                    if b.reg_type != D3DSPR_SAMPLER {
                        self.emit_mov_masked(&dest, &b, WRITEMASK_W, 0x0000_0000);
                    }
                    return dest;
                }
                let masks = [WRITEMASK_X, WRITEMASK_Y, WRITEMASK_Z, WRITEMASK_W];
                for (i, arg) in args.iter().take(4).enumerate() {
                    let src = self.compile_expr(arg);
                    if src.reg_type != D3DSPR_SAMPLER {
                        self.emit_mov_masked(&dest, &src, masks[i], 0x0000_0000);
                    }
                }
                dest
            }
            Expr::FunctionCall(name, args) => {
                let dest = self.alloc_temp();
                match name.as_str() {
                    "mul" if args.len() == 2 => {
                        // Prefer matrix shape from the *expression* (casts like
                        // `(float2x4)cBaseTextureTransform`) before falling back
                        // to register maps / component mul.
                        let rows_b = self.matrix_rows_of_expr(&args[1]);
                        let rows_a = self.matrix_rows_of_expr(&args[0]);
                        let a = self.compile_expr(&args[0]);
                        let b = self.compile_expr(&args[1]);
                        if let Some(rows) = rows_b.or_else(|| self.is_matrix_const(&b)) {
                            if rows >= 4 {
                                self.emit_instruction(OP_M4X4, &dest, &[a, b]);
                            } else if rows == 3 {
                                self.emit_instruction(OP_M4X3, &dest, &[a, b]);
                            } else {
                                self.emit_row_dots(&dest, &a, &b, rows);
                            }
                        } else if let Some(rows) = rows_a.or_else(|| self.is_matrix_const(&a)) {
                            if rows >= 4 {
                                self.emit_instruction(OP_M4X4, &dest, &[b, a]);
                            } else if rows == 3 {
                                self.emit_instruction(OP_M4X3, &dest, &[b, a]);
                            } else {
                                self.emit_row_dots(&dest, &b, &a, rows);
                            }
                        } else {
                            self.emit_instruction(OP_MUL, &dest, &[a, b]);
                        }
                    }
                    "mul4x3" if args.len() == 2 => {
                        let v = self.compile_expr(&args[0]);
                        let m = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_M4X3, &dest, &[v, m]);
                    }
                    "mul3x3" if args.len() == 2 => {
                        let v = self.compile_expr(&args[0]);
                        let m = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_M3X3, &dest, &[v, m]);
                    }
                    "dot" if args.len() == 2 => {
                        let a = self.compile_expr(&args[0]);
                        let b = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_DP4, &dest, &[a, b]);
                    }
                    "lerp" if args.len() == 3 => {
                        // HLSL lerp(a,b,f) == D3D LRP dest, f, b, a
                        let a = self.compile_expr(&args[0]);
                        let b = self.compile_expr(&args[1]);
                        let f = self.compile_expr(&args[2]);
                        self.emit_instruction(OP_LRP, &dest, &[f, b, a]);
                    }
                    // VGUI UnlitGeneric does GammaToLinear via pow(c, 2.2). Without
                    // this, unknown intrinsics fall through to MOV and sRGB write
                    // washes midtones into flat medium grays.
                    "pow" if args.len() == 2 => {
                        let base = self.compile_expr(&args[0]);
                        let exp = self.compile_expr(&args[1]);
                        self.emit_pow(&dest, &base, &exp);
                    }
                    // common_fxc.h helpers — dx9-rs does not yet expand user
                    // functions, so treat these as intrinsics (pow). Match
                    // float4 overloads: convert .xyz only, preserve .w (alpha).
                    // Applying pow to alpha crushed font coverage to 1px stems.
                    "GammaToLinear" if args.len() == 1 => {
                        let base = self.compile_expr(&args[0]);
                        let exp = self.def_const(2.2, 2.2, 2.2, 2.2);
                        self.emit_pow_rgb_preserve_alpha(&dest, &base, &exp);
                    }
                    "LinearToGamma" if args.len() == 1 => {
                        let base = self.compile_expr(&args[0]);
                        let exp = self.def_const(1.0 / 2.2, 1.0 / 2.2, 1.0 / 2.2, 1.0 / 2.2);
                        self.emit_pow_rgb_preserve_alpha(&dest, &base, &exp);
                    }
                    "log" | "log2" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_LOG, &dest, &[src]);
                    }
                    "exp" | "exp2" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_EXP, &dest, &[src]);
                    }
                    "rsqrt" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_RSQ, &dest, &[src]);
                    }
                    "rcp" | "rcp_safe" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_RCP, &dest, &[src]);
                    }
                    "sqrt" if args.len() == 1 => {
                        // D3D9 has no SQRT — rsq then rcp.
                        let src = self.compile_expr(&args[0]);
                        let tmp = self.alloc_temp();
                        self.emit_instruction(OP_RSQ, &tmp, &[src]);
                        self.emit_instruction(OP_RCP, &dest, &[tmp]);
                    }
                    "abs" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_ABS, &dest, &[src]);
                    }
                    "frac" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_FRC, &dest, &[src]);
                    }
                    "normalize" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        self.emit_instruction(OP_NRM, &dest, &[src]);
                    }
                    "saturate" if args.len() == 1 => {
                        let src = self.compile_expr(&args[0]);
                        let one = self.def_const(1.0, 1.0, 1.0, 1.0);
                        let zero = self.def_const(0.0, 0.0, 0.0, 0.0);
                        let tmp = self.alloc_temp();
                        self.emit_instruction(OP_MIN, &tmp, &[src, one]);
                        self.emit_instruction(OP_MAX, &dest, &[tmp, zero]);
                    }
                    "min" if args.len() == 2 => {
                        let a = self.compile_expr(&args[0]);
                        let b = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_MIN, &dest, &[a, b]);
                    }
                    "max" if args.len() == 2 => {
                        let a = self.compile_expr(&args[0]);
                        let b = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_MAX, &dest, &[a, b]);
                    }
                    // SkinPosition* non-skinned path is mul4x3(pos, cModel[0]).
                    // VGUI (and most UnlitGeneric) uses SkinPositionAndNormal;
                    // if cModel isn't committed the m4x3 yields zeros and every
                    // quad collapses → magenta clear only. Mov through for now;
                    // real skinning lands once we compile these properly.
                    "SkinPosition" if args.len() >= 5 => {
                        let pos = self.compile_expr(&args[1]);
                        let out = self.compile_expr(&args[4]);
                        self.emit_instruction(OP_MOV, &out, &[pos]);
                        return out;
                    }
                    "SkinPositionAndNormal" if args.len() >= 7 => {
                        let pos = self.compile_expr(&args[1]);
                        let normal = self.compile_expr(&args[2]);
                        let out_pos = self.compile_expr(&args[5]);
                        let out_n = self.compile_expr(&args[6]);
                        self.emit_instruction(OP_MOV, &out_pos, &[pos]);
                        self.emit_instruction(OP_MOV, &out_n, &[normal]);
                        return out_pos;
                    }
                    "SkinPositionNormalAndTangentSpace" if args.len() >= 9 => {
                        let pos = self.compile_expr(&args[1]);
                        let normal = self.compile_expr(&args[2]);
                        let tangent = self.compile_expr(&args[3]);
                        let out_pos = self.compile_expr(&args[6]);
                        let out_n = self.compile_expr(&args[7]);
                        let out_t = self.compile_expr(&args[8]);
                        self.emit_instruction(OP_MOV, &out_pos, &[pos]);
                        self.emit_instruction(OP_MOV, &out_n, &[normal]);
                        self.emit_instruction(OP_MOV, &out_t, &[tangent]);
                        return out_pos;
                    }
                    "tex2D" | "texCUBE" | "tex3D" if args.len() == 2 => {
                        let sampler = self.compile_expr(&args[0]);
                        let uv = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_TEX, &dest, &[uv, sampler]);
                    }
                    "tex2Dproj" | "texCUBEproj" | "tex3Dproj" if args.len() == 2 => {
                        let sampler = self.compile_expr(&args[0]);
                        let uv = self.compile_expr(&args[1]);
                        self.emit_instruction_flags(OP_TEX | TEXLD_PROJECT, &dest, &[uv, sampler]);
                    }
                    "tex2Dbias" | "texCUBEbias" | "tex3Dbias" if args.len() == 2 => {
                        let sampler = self.compile_expr(&args[0]);
                        let uv = self.compile_expr(&args[1]);
                        self.emit_instruction_flags(OP_TEX | TEXLD_BIAS, &dest, &[uv, sampler]);
                    }
                    "tex2Dlod" | "texCUBElod" | "tex3Dlod" if args.len() == 2 => {
                        let sampler = self.compile_expr(&args[0]);
                        let uv = self.compile_expr(&args[1]);
                        self.emit_instruction(OP_TEXLDL, &dest, &[uv, sampler]);
                    }
                    _ => {
                        if !args.is_empty() {
                            let src = self.compile_expr(&args[0]);
                            if src.reg_type != D3DSPR_SAMPLER {
                                self.emit_instruction(OP_MOV, &dest, &[src]);
                            }
                        }
                    }
                }
                dest
            }
            Expr::Cast(_, base) => self.compile_expr(base),
            Expr::Ternary(cond, then_expr, else_expr) => {
                if let Some(b) = self.eval_const_bool(cond) {
                    return if b {
                        self.compile_expr(then_expr)
                    } else {
                        self.compile_expr(else_expr)
                    };
                }
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
        self.emit_instruction_flags(opcode, dest, sources);
    }

    /// D3D9 `pow` is scalar (DXVK requires replicate swizzles and only uses .x).
    /// Emit one POW per component so `pow(float3, 2.2)` works.
    fn emit_pow(
        &mut self,
        dest: &ResolvedRegister,
        base: &ResolvedRegister,
        exp: &ResolvedRegister,
    ) {
        let comps = [
            (WRITEMASK_X, SWIZZLE_XXXX),
            (WRITEMASK_Y, SWIZZLE_YYYY),
            (WRITEMASK_Z, SWIZZLE_ZZZZ),
            (WRITEMASK_W, SWIZZLE_WWWW),
        ];
        for (mask, swizzle) in comps {
            self.bytecode.push((3 << 24) | OP_POW);
            self.bytecode
                .push(Self::encode_register(dest.reg_type, dest.index, mask));
            self.bytecode
                .push(Self::encode_register(base.reg_type, base.index, swizzle));
            // Exponent is almost always a scalar constant — replicate .x
            self.bytecode
                .push(Self::encode_register(exp.reg_type, exp.index, SWIZZLE_XXXX));
        }
    }

    /// Like `emit_pow`, but only on .xyz; .w is copied (common_fxc.h GammaToLinear).
    fn emit_pow_rgb_preserve_alpha(
        &mut self,
        dest: &ResolvedRegister,
        base: &ResolvedRegister,
        exp: &ResolvedRegister,
    ) {
        let comps = [
            (WRITEMASK_X, SWIZZLE_XXXX),
            (WRITEMASK_Y, SWIZZLE_YYYY),
            (WRITEMASK_Z, SWIZZLE_ZZZZ),
        ];
        for (mask, swizzle) in comps {
            self.bytecode.push((3 << 24) | OP_POW);
            self.bytecode
                .push(Self::encode_register(dest.reg_type, dest.index, mask));
            self.bytecode
                .push(Self::encode_register(base.reg_type, base.index, swizzle));
            self.bytecode
                .push(Self::encode_register(exp.reg_type, exp.index, SWIZZLE_XXXX));
        }
        // dest.w = base.w
        self.bytecode.push((2 << 24) | OP_MOV);
        self.bytecode.push(Self::encode_register(
            dest.reg_type,
            dest.index,
            WRITEMASK_W,
        ));
        self.bytecode.push(Self::encode_register(
            base.reg_type,
            base.index,
            SWIZZLE_WWWW,
        ));
    }

    fn emit_instruction_flags(
        &mut self,
        opcode_and_flags: u32,
        dest: &ResolvedRegister,
        sources: &[ResolvedRegister],
    ) {
        let size = 1 + sources.len() as u32; // dest + sources
        let inst_token = (size << 24) | opcode_and_flags;
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
