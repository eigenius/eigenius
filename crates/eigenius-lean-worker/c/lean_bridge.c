/*
 * Copyright 2026 The Eigenius Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
 * implied. See the License for the specific language governing
 * permissions and limitations under the License.
 */

/*
 * Implementation of the inline-thunks declared in `lean_bridge.h`.
 * Each thunk is a one-liner — its only role is to provide a
 * linkable symbol for what `lean.h` defines as `static inline`.
 *
 * Compiling this against `<lean/lean.h>` brings the inline bodies
 * into this translation unit; the wrapper functions are then real
 * (non-inline) symbols rustc can `extern "C"` declare.
 */

#include <lean/lean.h>

#include "lean_bridge.h"

EiLeanObj *ei_lean_alloc_byte_array(size_t size, size_t capacity) {
    return (EiLeanObj *)lean_alloc_sarray(1, size, capacity);
}

size_t ei_lean_sarray_size(EiLeanObj *arr) {
    return lean_sarray_size((lean_object *)arr);
}

uint8_t *ei_lean_sarray_cptr(EiLeanObj *arr) {
    return lean_sarray_cptr((lean_object *)arr);
}

EiLeanObj *ei_lean_io_result_mk_ok(EiLeanObj *val) {
    return (EiLeanObj *)lean_io_result_mk_ok((lean_object *)val);
}

EiLeanObj *ei_lean_io_result_mk_error(EiLeanObj *err) {
    return (EiLeanObj *)lean_io_result_mk_error((lean_object *)err);
}

EiLeanObj *ei_lean_io_mk_world(void) {
    return (EiLeanObj *)lean_io_mk_world();
}

EiLeanObj *ei_lean_box(size_t n) {
    return (EiLeanObj *)lean_box(n);
}

size_t ei_lean_unbox(EiLeanObj *o) {
    return lean_unbox((lean_object *)o);
}

EiLeanObj *ei_lean_box_usize(size_t v) {
    return (EiLeanObj *)lean_box_usize(v);
}

EiLeanExternalClass *ei_lean_register_external_class(EiLeanFinalizeProc finalize,
                                                    EiLeanForeachProc foreach) {
    /* Lean's `lean_external_foreach_proc` takes
     * `(void*, b_lean_obj_arg)`, where `b_lean_obj_arg` is
     * `lean_object*`. Our `EiLeanForeachProc` declares the second
     * arg as `EiLeanObj*` (void* alias). Casting through the
     * function-pointer type lets us pass the Rust callback
     * unchanged. */
    return (EiLeanExternalClass *)lean_register_external_class(
        finalize, (lean_external_foreach_proc)foreach);
}

EiLeanObj *ei_lean_alloc_external(EiLeanExternalClass *cls, void *data) {
    return (EiLeanObj *)lean_alloc_external((lean_external_class *)cls, data);
}

void *ei_lean_get_external_data(EiLeanObj *o) {
    return lean_get_external_data((lean_object *)o);
}
