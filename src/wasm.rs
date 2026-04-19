//! WASM bindings for the GX playground

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use crate::interpreter::Interpreter;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

/// Run GX source code and return captured output as a string.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn gx_run(source: &str) -> String {
    let program = match crate::parse_source(source) {
        Ok(p) => p,
        Err(e) => return format!("Parse error: {}", e),
    };

    let mut interp = Interpreter::new();
    interp.enable_capture();

    match interp.run_program(&program) {
        Ok(_) => interp.captured_output(),
        Err(e) => {
            let out = interp.captured_output();
            if out.is_empty() {
                format!("Error: {}", e)
            } else {
                format!("{}\nError: {}", out, e)
            }
        }
    }
}

/// Check GX source syntax. Returns empty string on success, error on failure.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn gx_check(source: &str) -> String {
    match crate::parse_source(source) {
        Ok(p) => format!(
            "OK — {} helper{}, {} import{}",
            p.helpers.len(),
            if p.helpers.len() == 1 { "" } else { "s" },
            p.imports.len(),
            if p.imports.len() == 1 { "" } else { "s" },
        ),
        Err(e) => format!("Error: {}", e),
    }
}
