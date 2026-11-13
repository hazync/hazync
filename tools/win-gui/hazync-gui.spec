# PyInstaller spec — one Hazync.exe, no Python install required.
#
#     pip install pyinstaller
#     pyinstaller --clean --noconfirm hazync-gui.spec
#     dist/Hazync/Hazync.exe          (the folder installer/hazync.iss packs into Hazync-Setup.exe)
#
# ⛔ WHY THIS EXISTS. The whole point of the GUI is reaching people who do not use a terminal, and
# "first install Python, then pip install cryptography" undoes that before they start. A single exe
# is the difference between a tool contributors can use and one only developers can.
#
# ⛔⛔ winshim MUST BE SHIPPED AS A DATA FILE, NOT IMPORTED. It is `fcntl.py`, and the GUI puts its
# DIRECTORY on PYTHONPATH for the worker's child process — a separate interpreter. If PyInstaller
# bundled it as a module instead, there would be no directory on disk to point at, and the worker
# would die on `import fcntl` exactly as it does today without the shim.
#
# ⚠ AND THE GUI ITSELF MUST NOT IMPORT IT. A frozen app on Windows has no real `fcntl` either, but
# nothing in the GUI needs one; only the worker does. Shipping it as data keeps that separation.
#
# ⚠ `cryptography` is collected because the WORKER needs it to sign claims. When the worker runs
# under the frozen interpreter it must find it; when it runs under a system Python it uses that
# one. Both paths work, and the Setup tab reports which.

import ast
import os

block_cipher = None

# ⛔ THE WORKER RUNS INSIDE THIS BUNDLE (`Hazync.exe <hazync-worker> run`, frozenpy.py), and PyInstaller bundles only
# what the APP imports: a stdlib module only the worker uses (concurrent.futures, secrets, socket...) would be missing
# and the worker would die on import, on the user's machine. So the worker's own imports are read from its source
# (coordinator/hazync, shipped as hazync-worker) at build time; a new import there is bundled with no edit here.
# fcntl is not a module on Windows: winshim provides it (below).
_WORKER = os.path.join(os.getcwd(), "..", "..", "coordinator", "hazync")
_worker_imports = set()
for _n in ast.walk(ast.parse(open(_WORKER, encoding="utf-8").read())):
    if isinstance(_n, ast.Import):
        _worker_imports |= {_a.name for _a in _n.names}
    elif isinstance(_n, ast.ImportFrom) and _n.module and _n.level == 0:
        _worker_imports.add(_n.module)
_worker_imports -= {"fcntl"}

a = Analysis(
    ["hazync_gui.py"],
    pathex=[os.getcwd()],
    binaries=[],
    datas=[
        # the shim, as a real directory the worker's PYTHONPATH can point at
        ("winshim/fcntl.py", "winshim"),
        # the honest state of things travels with the program, not just the repo
        ("README.md", "."),
    ],
    hiddenimports=[
        # imported by path at runtime rather than statically, so PyInstaller cannot see them
        "supervisor", "hazync_api", "brand", "firstrun", "frozenpy",
        # the worker signs with ed25519; collected so a frozen build can satisfy it
        "cryptography", "cryptography.hazmat.primitives.asymmetric.ed25519",
    ] + sorted(_worker_imports),
    hookspath=[],
    runtime_hooks=[],
    # ⚠ Nothing excluded. A GUI that crashes on a missing stdlib module to save 3 MB is a bad trade.
    excludes=[],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

# ONE FOLDER, not one file (2026-11-13, the installer): Hazync-Setup.exe installs this folder. A one-file exe unpacks
# itself to %TEMP% on every start, which is slow, leaves copies behind when killed, and is what antivirus heuristics
# flag; and the worker is started as `Hazync.exe <worker>` (frozenpy.py), so it would unpack once more per worker.
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="Hazync",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,          # ⚠ UPX-packed exes are a favourite of antivirus heuristics. Not worth it.
    runtime_tmpdir=None,
    # ⛔ console=False, or a black window sits behind the app for the whole session.
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    # ⚠ No icon yet: the brand asset is an SVG and Windows wants .ico. brand.py draws the logo
    # inside the window; converting it for the taskbar is a small follow-up, not a blocker.
)

coll = COLLECT(
    exe,
    a.binaries,
    a.zipfiles,
    a.datas,
    strip=False,
    upx=False,
    name="Hazync",
)
