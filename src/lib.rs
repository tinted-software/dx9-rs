pub mod ast;
pub mod codegen;
pub mod lexer;
pub mod parser;

use crate::codegen::Codegen;
use crate::parser::Parser;
use std::slice;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn compile_dx9_shader(
    source_ptr: *const u8,
    source_len: usize,
    is_pixel: bool,
    out_bytecode_ptr: *mut *mut u8,
    out_bytecode_len: *mut usize,
) -> i32 {
    let source_slice = unsafe { slice::from_raw_parts(source_ptr, source_len) };
    let source = String::from_utf8_lossy(source_slice);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parser = Parser::new(&source, "ffi_shader.hlsl");
        let ast = parser.parse();
        let codegen = Codegen::new();
        let bytecode = codegen.compile(&ast, is_pixel);

        let mut binary_data = Vec::with_capacity(bytecode.len() * 4);
        for word in bytecode {
            binary_data.extend_from_slice(&word.to_le_bytes());
        }
        binary_data.into_boxed_slice()
    }));

    match result {
        Ok(boxed_slice) => {
            let len = boxed_slice.len();

            let raw_ptr = Box::into_raw(boxed_slice) as *mut u8;

            unsafe {
                *out_bytecode_ptr = raw_ptr;
                *out_bytecode_len = len;
            }
            0
        }
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
