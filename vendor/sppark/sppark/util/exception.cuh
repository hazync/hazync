// Copyright Supranational LLC
// Licensed under the Apache License, Version 2.0, see LICENSE for details.
// SPDX-License-Identifier: Apache-2.0

#ifndef __SPPARK_UTIL_EXCEPTION_CUH__
#define __SPPARK_UTIL_EXCEPTION_CUH__

#include "exception.hpp"

using cuda_error = sppark_error;

// HAZYNC_631_REPORT_BEFORE_THROW — hazync#631.
//
// ⛔⛔ ON MSVC THIS THROW DESTROYS THE ONLY COPY OF THE ERROR. `throw` from C++ into Rust is a
// FOREIGN (SEH) exception, so the Rust runtime aborts immediately:
//
//     fatal runtime error: Rust cannot catch foreign exceptions, aborting
//
// No Rust handler runs, which means main.rs's OOM advice — the handler that is supposed to print
// `HAZYNC_SEG_PO2=<value to try>` — can NEVER fire on Windows. Every CUDA failure produces the same
// nine words and nothing else.
//
// 📏 Measured 2026-10-04 on a real GTX 1050 Ti, the first time anyone ran the Windows CUDA build on
// a GPU: `method-id` printed the canonical id and `regress` passed, then `prove_segment_core` — the
// FIRST GPU call — aborted with exactly that message. Diagnosis then had to proceed by elimination,
// and two plausible theories (out of memory; a PTX/arch mismatch) were both wrong. The error was
// sitting right here the whole time and was thrown away.
//
// ⇒ PRINT IT, THEN THROW. One line, MSVC only, and it changes nothing about control flow: the
// exception still propagates exactly as before on every platform. On Linux the throw is caught
// normally, so this stays out of the way there.
//
// ⚠ Deliberately NOT a port of sppark's RustError plumbing. Converting these at the FFI boundary is
// the right long-term fix and is a much larger change across risc0's CUDA sys crates; this makes
// the next Windows failure diagnosable today without touching how anything behaves.
#if defined(_MSC_VER) && !defined(__clang__)
# define HAZYNC_631_REPORT_CUDA_ERROR(s) do {               \
    std::fprintf(stderr, "\nCUDA ERROR: %s\n", (s).c_str()); \
    std::fprintf(stderr, "  (the abort that follows is this error crossing C++ -> Rust on MSVC;\n" \
                         "   if it mentions out of memory, retry with a lower HAZYNC_SEG_PO2)\n"); \
    std::fflush(stderr);                                    \
} while(0)
#else
# define HAZYNC_631_REPORT_CUDA_ERROR(s) ((void)0)
#endif

// HAZYNC_631_PENDING_TRACE (HAZYNC_CUDA_TRACE=1) — hazync#631. The GTX 1050 Ti trace showed an
// "operation not supported" (801) ALREADY PENDING before the first NTT launch: some earlier call
// failed and nobody checked it. Reading the pending error before every CUDA_OK brackets the culprit
// between this check and the previous one, by file and line. Prints only when something is pending.
#include <cstdlib>
static inline void hazync_631_pending(const char* what, const char* file, int line)
{
    static const bool on = std::getenv("HAZYNC_CUDA_TRACE") != nullptr;
    if (!on) return;
    cudaError_t e = cudaGetLastError();
    if (e != cudaSuccess) {
        std::fprintf(stderr, "HAZYNC TRACE pending error %d (%s) found BEFORE %s @ %s:%d\n",
                     (int)e, cudaGetErrorString(e), what, file, line);
        std::fflush(stderr);
    }
}

#define CUDA_OK(expr) do {                                  \
    hazync_631_pending(#expr, __FILE__, __LINE__);          \
    cudaError_t code = expr;                                \
    if (code != cudaSuccess) {                              \
        auto file = std::strstr(__FILE__, "sppark");        \
        auto str = fmt("%s@%s:%d failed: \"%s\"", #expr,    \
                       file ? file : __FILE__, __LINE__,    \
                       cudaGetErrorString(code));           \
        HAZYNC_631_REPORT_CUDA_ERROR(str);                  \
        throw cuda_error{-code, str};                       \
    }                                                       \
} while(0)

#endif
