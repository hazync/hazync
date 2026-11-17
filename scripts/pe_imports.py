#!/usr/bin/env python3
"""List a Windows exe's imported DLLs, and fail when it needs one Windows does not have (review of a04038ce, M1).

    scripts/pe_imports.py <host.exe>             # print the imports, exit 0
    scripts/pe_imports.py <host.exe> --cpu       # exit 1 if it needs ANY non-system DLL (the CPU host ships alone)
    scripts/pe_imports.py <host.exe> --cuda      # the same, allowing the driver's nvcuda.dll and cudart64_*.dll
    scripts/pe_imports.py <host.exe> --beside    # exit 1 unless every non-system DLL sits beside the exe (an install)
    scripts/pe_imports.py <host.exe> --vc-runtime  # print only the Visual C++ runtime DLLs it imports (for copying)

The PE reader and the allow-list are the Windows app's own (tools/win-gui/supervisor.py: imported_dlls, SYSTEM_DLLS),
so the release gate and the app agree on what "starts on a fresh PC" means. Runs anywhere; no Windows needed.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tools", "win-gui"))
import supervisor  # noqa: E402


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__.strip())
        return 0 if argv else 2
    path, mode = argv[0], (argv[1] if len(argv) > 1 else "")
    dlls = supervisor.imported_dlls(path)
    if not dlls:
        print(f"FAIL {path}: no import table read (not a PE, or unreadable)")
        return 1
    if mode == "--vc-runtime":
        for d in supervisor.foreign_imports(dlls, cuda=True):
            if d.lower().startswith(("vcruntime", "msvcp", "vcomp", "concrt")):
                print(d)
        return 0
    print(f"{os.path.basename(path)} imports: " + ", ".join(sorted(set(d.lower() for d in dlls))))
    if mode == "--beside":
        here = {f.lower() for f in os.listdir(os.path.dirname(os.path.abspath(path)))}
        missing = [d for d in supervisor.foreign_imports(dlls, cuda=True)
                   if d.lower() not in here and d.lower() != "nvcuda.dll"]
        if missing:
            print(f"FAIL {path} needs DLLs that are neither part of Windows nor beside it: {', '.join(missing)}")
            return 1
        print("  ok   every DLL it needs is part of Windows, the NVIDIA driver's, or beside it")
        return 0
    if mode not in ("--cpu", "--cuda"):
        return 0
    bad = supervisor.foreign_imports(dlls, cuda=(mode == "--cuda"))
    if bad:
        print(f"FAIL {path} needs DLLs Windows does not ship: {', '.join(bad)}"
              + (" (the Visual C++ runtime: build with -C target-feature=+crt-static)"
                 if any(d.lower().startswith(("vcruntime", "msvcp", "vcomp", "concrt")) for d in bad) else ""))
        return 1
    print(f"  ok   only system DLLs{' (plus the NVIDIA driver and cudart)' if mode == '--cuda' else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
