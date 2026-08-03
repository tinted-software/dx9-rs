pub mod ast;
pub mod codegen;
pub mod lexer;
pub mod parser;
pub mod preprocess;

use crate::codegen::Codegen;
use crate::parser::Parser;
use crate::preprocess::{PreprocessOptions, preprocess};
use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::PathBuf;
use std::slice;

/// NULL-terminated D3DXMACRO-compatible entry.
#[repr(C)]
pub struct Dx9Macro {
    pub name: *const c_char,
    pub definition: *const c_char,
}

/// Compile HLSL to DX9 SM3 bytecode.
///
/// `macros` is an optional NULL-terminated list of (name, definition) C string pairs,
/// same layout as D3DXMACRO. `include_dir` is an optional search path for `#include`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compile_dx9_shader(
    source_ptr: *const u8,
    source_len: usize,
    is_pixel: bool,
    profile: *const c_char,
    macros: *const Dx9Macro,
    include_dir: *const c_char,
    out_bytecode_ptr: *mut *mut u8,
    out_bytecode_len: *mut usize,
) -> i32 {
    let source_slice = unsafe { slice::from_raw_parts(source_ptr, source_len) };
    let source = String::from_utf8_lossy(source_slice);

    let mut options = PreprocessOptions::default();

    if !profile.is_null() {
        let profile = unsafe { CStr::from_ptr(profile) }.to_string_lossy();
        // vs_3_0 → SHADER_MODEL_VS_3_0=1
        let model = profile.replace('.', "_").to_ascii_uppercase();
        options
            .defines
            .insert(format!("SHADER_MODEL_{model}"), "1".into());
    } else if is_pixel {
        options
            .defines
            .insert("SHADER_MODEL_PS_3_0".into(), "1".into());
    } else {
        options
            .defines
            .insert("SHADER_MODEL_VS_3_0".into(), "1".into());
    }

    if !macros.is_null() {
        let mut ptr = macros;
        loop {
            let m = unsafe { &*ptr };
            if m.name.is_null() {
                break;
            }
            let name = unsafe { CStr::from_ptr(m.name) }
                .to_string_lossy()
                .into_owned();
            let def = if m.definition.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(m.definition) }
                    .to_string_lossy()
                    .into_owned()
            };
            options.defines.insert(name, def);
            ptr = unsafe { ptr.add(1) };
        }
    }

    if !include_dir.is_null() {
        let dir = unsafe { CStr::from_ptr(include_dir) }.to_string_lossy();
        if !dir.is_empty() {
            options.include_dirs.push(PathBuf::from(dir.as_ref()));
        }
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let preprocessed = match preprocess(&source, &options) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("dx9-rs preprocess error: {e}");
                return Err(-1);
            }
        };
        let mut parser = Parser::new(&preprocessed, "ffi_shader.hlsl");
        let ast = parser.parse();
        let codegen = Codegen::new();
        let is_ps = is_pixel
            || options
                .defines
                .keys()
                .any(|k| k.starts_with("SHADER_MODEL_PS_"));
        let bytecode = codegen.compile(&ast, is_ps);

        let mut binary_data = Vec::with_capacity(bytecode.len() * 4);
        for word in bytecode {
            binary_data.extend_from_slice(&word.to_le_bytes());
        }
        Ok(binary_data.into_boxed_slice())
    }));

    match result {
        Ok(Ok(boxed_slice)) => {
            let len = boxed_slice.len();
            let raw_ptr = Box::into_raw(boxed_slice) as *mut u8;
            unsafe {
                *out_bytecode_ptr = raw_ptr;
                *out_bytecode_len = len;
            }
            0
        }
        Ok(Err(code)) => code,
        Err(_) => -2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_dx9_shader_bytecode(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        let slice = unsafe { slice::from_raw_parts_mut(ptr, len) };
        let _ = unsafe { Box::from_raw(slice) };
    }
}
