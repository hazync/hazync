// Does THIS card, on THIS platform, actually support what sppark's NTT needs? (hazync#631)
//
// ⛔⛔ WHY THIS EXISTS. #631 has had FOUR "root causes" in three days -- OOM, the device floor,
// memory pools, and missing sm_61 kernels -- and three of them were refuted by something already
// written in the repo. Every one of those rounds cost a 2h16m risc0+sppark build and most of an
// evening. This file compiles in seconds, downloads in a blink, and answers the question directly
// instead of inferring it from a prover that fails for any of a dozen reasons.
//
// It prints ERROR NAMES, not just messages. That distinction is the whole point: a missing kernel
// image is cudaErrorNoKernelImageForDevice, while a refused cooperative launch is
// cudaErrorNotSupported -- whose message is the bare words "operation not supported", exactly what
// the real prover reports. Matching the message alone is how a wrong theory survived a whole round.
//
//     probe.exe                  # report every device, then run each launch test
//     probe.exe path\to\host.exe  # ...and then load the PROVER'S OWN GPU code onto each device
//
// ⭐⭐ THE SECOND FORM IS THE ONE THAT MATTERS NOW. Every test in the first form passes on a real
// GTX 1050 Ti, so the card can do each operation the prover needs -- and the prover still fails on
// it. What the first form cannot say is whether the card will accept the prover's ACTUAL kernels,
// because it only ever launches two trivial ones of its own. The second form finds every CUDA fat
// binary embedded in the host executable and registers each with the CUDA runtime, as the prover's
// own start-up does, with module loading set to EAGER so each is loaded at once. It first says
// whether the binary holds code built for this card at all, then loads each finished module in a
// fresh process and reports the error NAME for whatever is refused.
//
// ⚠ WHAT IT CANNOT DO, measured rather than assumed: a COMPRESSED module "loads" here even when
// deliberately damaged, so for those it reports CANNOT TELL instead of a pass. In the Windows build
// of 2026-10-05 that is two of the five modules.
//
// Exit code is 0 whatever the GPU says: a card that cannot do something is an ANSWER, not a
// failure of this program. Only a probe that could not run at all exits non-zero.

#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include <cooperative_groups.h>

// Deliberately trivial. If a launch fails it must be because of HOW it was launched or WHAT the
// card is, never because the kernel did something exotic.
__global__ void touch(int* out)
{
    if (threadIdx.x == 0 && blockIdx.x == 0)
        *out = 1;
}

// The shape sppark's NTT actually uses: a grid-wide sync, which REQUIRES a cooperative launch.
// (vendor/sppark/sppark/ntt/kernels.cu:209 does exactly this.)
__global__ void touch_grid_sync(int* out)
{
    cooperative_groups::this_grid().sync();
    if (threadIdx.x == 0 && blockIdx.x == 0)
        *out = 2;
}

// ── the prover's real kernels ─────────────────────────────────────────────────────────────────────
//
// A CUDA "fat binary" is what nvcc embeds for each compiled .cu file: one blob holding a compiled
// image per target architecture (and sometimes PTX to compile on the spot). It begins with a fixed
// 16-byte header, which is ALL this code relies on to find one:
//
//     u32 magic = 0xBA55ED50    u16 version = 1    u16 header_size = 16    u64 payload_size
//
// ⚠ THE ENTRY TABLE BELOW IT IS DECODED FOR DISPLAY ONLY. Its layout is not a documented interface,
// so the per-entry lines (kind, architecture, compressed or not) are labelled best-effort and
// nothing is decided from them. The verdict comes from the DRIVER accepting or refusing the blob,
// which depends on no layout knowledge of ours beyond those 16 bytes.
static const uint32_t FATBIN_MAGIC = 0xBA55ED50u;

struct FatHeader { uint32_t magic; uint16_t version; uint16_t header_size; uint64_t size; };
struct FatEntry {
    uint16_t kind; uint16_t unknown1; uint32_t header_size; uint64_t size;
    uint32_t compressed_size; uint32_t unknown2; uint16_t minor; uint16_t major;
    uint32_t arch; uint32_t obj_name_offset; uint32_t obj_name_len;
    uint64_t flags; uint64_t zero; uint64_t decompressed_size;
};

static bool read_file(const char* path, std::vector<unsigned char>& out)
{
    FILE* f = std::fopen(path, "rb");
    if (!f) return false;
    unsigned char buf[1 << 16];
    size_t n;
    while ((n = std::fread(buf, 1, sizeof buf, f)) > 0) out.insert(out.end(), buf, buf + n);
    std::fclose(f);
    return true;
}

// Every plausible fat binary in the file, as (offset, total bytes including the header).
static std::vector<std::pair<size_t, size_t>> find_fatbins(const std::vector<unsigned char>& b)
{
    std::vector<std::pair<size_t, size_t>> out;
    if (b.size() < sizeof(FatHeader)) return out;
    for (size_t i = 0; i + sizeof(FatHeader) <= b.size(); i += 4) {      // 4-byte aligned in practice
        FatHeader h;
        std::memcpy(&h, &b[i], sizeof h);
        // ⚠ All four fields, not the magic alone: four bytes will occur by chance somewhere in
        // 180 MB, and a false hit handed to the driver would be reported as a refused module.
        if (h.magic != FATBIN_MAGIC || h.version != 1 || h.header_size != sizeof(FatHeader)) continue;
        if (h.size < sizeof(FatEntry) || h.size > b.size() - i - sizeof(FatHeader)) continue;
        out.push_back({ i, (size_t)h.size + sizeof(FatHeader) });
        i += ((size_t)h.size + sizeof(FatHeader)) / 4 * 4;
        if (i >= 4) i -= 4;
    }
    return out;
}

// Where in the file the LINKED GPU modules live: (offset, size) of the section nvcc's linker step
// writes them to, or (0, 0) if the file is neither ELF nor PE or has no such section.
//
// ⛔ THIS DISTINCTION IS WHAT MADE THE SECOND VERSION WRONG. The prover holds 52 fat binaries, and
// only 5 are finished modules. The other 47 are RELOCATABLE PARTS -- the pieces those 5 were linked
// from, kept in a section of their own -- and a part registered by itself is refused on a card
// where the prover works perfectly (measured on an A40: "named symbol not found"). Testing them
// alone says nothing about the card. ELF calls the section `.nv_fatbin`; PE truncates names to
// eight characters, so there it is `.nv_fatb`.
static std::pair<size_t, size_t> linked_section(const std::vector<unsigned char>& b)
{
    auto u16 = [&](size_t o) { uint16_t v; std::memcpy(&v, &b[o], 2); return v; };
    auto u32 = [&](size_t o) { uint32_t v; std::memcpy(&v, &b[o], 4); return v; };
    auto u64 = [&](size_t o) { uint64_t v; std::memcpy(&v, &b[o], 8); return v; };
    if (b.size() > 64 && !std::memcmp(&b[0], "\x7f" "ELF", 4) && b[4] == 2) {          // ELF64
        size_t shoff = (size_t)u64(0x28), entsz = u16(0x3A), num = u16(0x3C), strndx = u16(0x3E);
        if (!shoff || entsz < 64 || shoff + num * entsz > b.size() || strndx >= num) return { 0, 0 };
        size_t stroff = (size_t)u64(shoff + strndx * entsz + 0x18);
        for (size_t i = 0; i < num; i++) {
            size_t sh = shoff + i * entsz;
            size_t name = stroff + u32(sh);
            if (name + 11 <= b.size() && !std::memcmp(&b[name], ".nv_fatbin", 11))
                return { (size_t)u64(sh + 0x18), (size_t)u64(sh + 0x20) };
        }
    } else if (b.size() > 0x40 && b[0] == 'M' && b[1] == 'Z') {                         // PE
        size_t pe = u32(0x3C);
        if (pe + 24 > b.size() || std::memcmp(&b[pe], "PE\0\0", 4)) return { 0, 0 };
        size_t num = u16(pe + 6), sh = pe + 24 + u16(pe + 20);
        for (size_t i = 0; i < num && sh + (i + 1) * 40 <= b.size(); i++)
            if (!std::memcmp(&b[sh + i * 40], ".nv_fatb", 8))
                return { (size_t)u32(sh + i * 40 + 20), (size_t)u32(sh + i * 40 + 16) };
    }
    return { 0, 0 };
}

// The architectures a fat binary holds a COMPILED image for, e.g. {61} or {80, 86, 89}. Best-effort
// (see the note on the entry table above), and corroborated: `cuobjdump` reports the same list.
static std::vector<unsigned> image_archs(const unsigned char* fat, size_t total, bool* has_ptx, bool* compressed)
{
    std::vector<unsigned> out;
    size_t off = sizeof(FatHeader);
    while (off + sizeof(FatEntry) <= total) {
        FatEntry e;
        std::memcpy(&e, fat + off, sizeof e);
        if (e.header_size < sizeof(FatEntry) || e.header_size > 4096 || e.size == 0 ||
            e.size > total - off) break;
        if (e.kind == 2) { out.push_back(e.arch); if (compressed && (e.flags & 0x2000)) *compressed = true; }
        if (e.kind == 1 && has_ptx) *has_ptx = true;
        off += (size_t)e.header_size + (size_t)e.size;
    }
    return out;
}

static void describe_entries(const unsigned char* fat, size_t total)
{
    size_t off = sizeof(FatHeader);
    int k = 0;
    while (off + sizeof(FatEntry) <= total && k < 32) {
        FatEntry e;
        std::memcpy(&e, fat + off, sizeof e);
        if (e.header_size < sizeof(FatEntry) || e.header_size > 4096 || e.size == 0 ||
            e.size > total - off) break;                     // not the layout we expected: stop
        std::printf("        entry %d (best-effort decode): %s for sm_%u, %llu bytes%s\n", k,
                    e.kind == 2 ? "compiled image" : e.kind == 1 ? "PTX source" : "unknown kind",
                    e.arch, (unsigned long long)e.size,
                    (e.flags & 0x2000) ? ", compressed" : "");
        off += (size_t)e.header_size + (size_t)e.size;
        k++;
    }
    if (k == 0) std::printf("        (entry table not decoded -- the verdict above still stands)\n");
}

// ⛔⛔ THE PROVER'S CODE MUST REACH THE DRIVER THE WAY THE PROVER SENDS IT, OR THE ANSWER IS FALSE.
// The first version of this handed each fat binary straight to the driver (cuModuleLoadFatBinary).
// Run on a rented A40 where the prover WORKS, it reported 42 refusals: the images are compressed by
// the toolkit that built them, the CUDA *runtime* linked into the prover unpacks them, and a driver
// older than that toolkit cannot unpack them itself. That is precisely the 1050 Ti's situation
// (toolkit 12.9, driver 12.7), so it would have "found" a cause that is not one.
//
// So this registers each fat binary with the RUNTIME, through the same three entry points every
// nvcc-compiled program calls from its static initialisers. They are not a documented api, but
// they are the ABI nvcc's own generated code depends on, so they cannot change under a toolkit.
struct FatWrapper { int magic; int version; const unsigned long long* data; void* extra; };
extern "C" void** __cudaRegisterFatBinary(void* wrapper);
extern "C" void __cudaRegisterFatBinaryEnd(void** handle);

// The child half: register the chosen fat binaries, then make the runtime build a context with
// module loading set to EAGER, so every registered image is loaded NOW rather than at first launch.
// Prints exactly one RESULT line. `which` < 0 means all of them.
static int child(const char* host, int which, int device, int flip)
{
    std::vector<unsigned char> image;
    if (!read_file(host, image)) { std::printf("RESULT cannot-read-host\n"); return 0; }
    auto fats = find_fatbins(image);
    std::vector<void*> keep;
    for (size_t m = 0; m < fats.size(); m++) {
        if (which >= 0 && (size_t)which != m) continue;
        // 8-byte aligned and never freed: the runtime keeps the pointer for the life of the process.
        size_t words = (fats[m].second + 7) / 8;
        unsigned long long* copy = (unsigned long long*)std::calloc(words, 8);
        std::memcpy(copy, &image[fats[m].first], fats[m].second);
        // The self-test's deliberate damage: a module the runtime must refuse.
        // ⚠ EVERY image in it, not one spot: a fat binary holds one image per architecture and
        // the runtime reads only the one for this card, so damage in the middle of the blob can
        // land in an image nobody loads -- which made the first self-test pass a damaged module.
        if (flip) {
            unsigned char* c = (unsigned char*)copy;
            size_t off = sizeof(FatHeader), total = fats[m].second;
            while (off + sizeof(FatEntry) <= total) {
                FatEntry e;
                std::memcpy(&e, c + off, sizeof e);
                if (e.header_size < sizeof(FatEntry) || e.header_size > 4096 || e.size == 0 ||
                    e.size > total - off) break;
                size_t body = off + e.header_size;
                for (size_t i = body + e.size / 4; i < body + e.size / 4 + 256 && i < body + e.size; i++)
                    c[i] ^= 0xA5;
                for (size_t i = body; i < body + 64 && i < body + e.size; i++)
                    c[i] ^= 0xA5;
                off = body + (size_t)e.size;
            }
        }
        FatWrapper* w = (FatWrapper*)std::calloc(1, sizeof(FatWrapper));
        w->magic = 0x466243b1; w->version = 1; w->data = copy; w->extra = nullptr;
        void** h = __cudaRegisterFatBinary(w);
        __cudaRegisterFatBinaryEnd(h);
        keep.push_back(copy);
    }
    // First failure wins, and it is reported with the call that returned it.
    const char* at = "";
    cudaError_t e = cudaSetDevice(device);
    if (e != cudaSuccess) at = "cudaSetDevice";
    int* d = nullptr;
    if (e == cudaSuccess) { e = cudaMalloc(&d, 4); if (e != cudaSuccess) at = "cudaMalloc"; }
    if (e == cudaSuccess) { touch<<<1, 32>>>(d); e = cudaGetLastError(); if (e != cudaSuccess) at = "first launch"; }
    if (e == cudaSuccess) { e = cudaDeviceSynchronize(); if (e != cudaSuccess) at = "cudaDeviceSynchronize"; }
    if (e == cudaSuccess) std::printf("RESULT OK %zu registered\n", keep.size());
    else std::printf("RESULT %s at %s: %s\n", cudaGetErrorName(e), at, cudaGetErrorString(e));
    std::fflush(stdout);
    return 0;
}

// Run this same program as a child and return its RESULT line (without the word RESULT).
static std::string run_child(const char* self, const char* host, int which, int device, int flip)
{
    char cmd[4096];
#ifdef _WIN32
    // cmd.exe strips one outer pair of quotes, so a command that begins with a quoted path needs a
    // second pair around the whole thing or a path with a space in it is cut at the space.
    std::snprintf(cmd, sizeof cmd, "\"\"%s\" --child \"%s\" %d %d %d\"", self, host, which, device, flip);
    FILE* p = _popen(cmd, "r");
#else
    std::snprintf(cmd, sizeof cmd, "\"%s\" --child \"%s\" %d %d %d", self, host, which, device, flip);
    FILE* p = popen(cmd, "r");
#endif
    if (!p) return "could-not-start-child";
    std::string out, line;
    char buf[1024];
    while (std::fgets(buf, sizeof buf, p)) {
        line = buf;
        if (line.rfind("RESULT ", 0) == 0) out = line.substr(7);
    }
#ifdef _WIN32
    int rc = _pclose(p);
#else
    int rc = pclose(p);
#endif
    while (!out.empty() && (out.back() == '\n' || out.back() == '\r')) out.pop_back();
    // ⛔ A child that died without a RESULT line is itself the finding -- on Windows a CUDA error
    // thrown across the language boundary kills the process -- so it is reported, not swallowed.
    if (out.empty()) { std::snprintf(buf, sizeof buf, "CHILD DIED with no result (exit status %d)", rc); out = buf; }
    return out;
}

// Load the prover's finished GPU modules onto one device. Returns the number refused, or -1 if
// nothing could be judged.
static int load_real_kernels(const char* self, const char* host, const std::vector<unsigned char>& image,
                             const std::vector<std::pair<size_t, size_t>>& fats, int device,
                             int cc_major, int cc_minor, bool verbose)
{
    auto sec = linked_section(image);
    std::vector<size_t> linked;
    for (size_t m = 0; m < fats.size(); m++)
        if (sec.second && fats[m].first >= sec.first && fats[m].first < sec.first + sec.second)
            linked.push_back(m);
    if (linked.empty()) {
        std::printf("  could not find the section holding the finished GPU modules, so which of the %zu\n"
                    "  fat binaries are modules and which are parts is unknown. Nothing was tested.\n", fats.size());
        return -1;
    }
    std::printf("  %zu finished module(s), %zu relocatable part(s) (parts are not tested: alone they are\n"
                "  refused even on a card where the prover works)\n\n", linked.size(), fats.size() - linked.size());

    // ⭐ FIRST, THE QUESTION THAT NEEDS NO GPU CALL AT ALL: does the binary hold code built for this
    // card? A compiled image runs only on its own generation -- an sm_61 image on 6.1 and later 6.x,
    // never on 7.x or 8.x -- so a card with no image of its own generation has nothing to run
    // unless the driver compiles the PTX on the spot.
    unsigned mine = (unsigned)(cc_major * 10 + cc_minor);
    int no_native = 0;
    for (size_t m : linked) {
        bool ptx = false, comp = false;
        auto archs = image_archs(&image[fats[m].first], fats[m].second, &ptx, &comp);
        bool native = false;
        std::string list;
        for (unsigned a : archs) {
            if (a / 10 == mine / 10 && a <= mine) native = true;
            list += (list.empty() ? "sm_" : ", sm_") + std::to_string(a);
        }
        if (!native) no_native++;
        std::printf("  module %2zu  %8zu bytes  images: %-28s %s%s\n", m, fats[m].second,
                    list.empty() ? "(none decoded)" : list.c_str(),
                    native ? "has code for this card" : "NO CODE FOR THIS CARD",
                    native ? "" : (ptx ? " (PTX present: would be compiled on the spot)" : " (and no PTX)"));
    }
    if (no_native)
        std::printf("  %d of %zu modules hold no image for compute %d.%d. This BUILD was not made for this card.\n",
                    no_native, linked.size(), cc_major, cc_minor);
    std::printf("\n");

    // ⛔ THEN LOAD THEM, AND PROVE PER MODULE THAT THE TEST CAN FAIL. Each module is first loaded
    // with every image in it deliberately damaged. If the runtime does not refuse THAT, it is not
    // really reading this module here -- measured on an A40, a compressed module "loads" damaged
    // or not -- and its verdict is reported as CANNOT TELL instead of as a pass.
    int refused = 0, judged = 0;
    for (size_t m : linked) {
        std::string damaged = run_child(self, host, (int)m, device, 1);
        std::string intact = run_child(self, host, (int)m, device, 0);
        bool can_fail = damaged.rfind("OK", 0) != 0, ok = intact.rfind("OK", 0) == 0;
        if (!can_fail) {
            std::printf("  module %2zu  CANNOT TELL -- a damaged copy also loads, so this method does not see inside it\n", m);
        } else {
            judged++;
            if (!ok) refused++;
            std::printf("  module %2zu  %s%s\n", m, ok ? "LOADS" : "REFUSED: ", ok ? "" : intact.c_str());
        }
        if (verbose || (can_fail && !ok)) describe_entries(&image[fats[m].first], fats[m].second);
    }
    std::printf("  judged %d of %zu module(s): %d load, %d refused\n", judged, linked.size(), judged - refused, refused);
    return judged ? refused : -1;
}

static void say(const char* what, cudaError_t e)
{
    // ⭐ NAME FIRST. "operation not supported" is ambiguous prose; cudaErrorNotSupported is not.
    std::printf("  %-34s %-34s %s\n", what,
                e == cudaSuccess ? "OK" : cudaGetErrorName(e),
                e == cudaSuccess ? "" : cudaGetErrorString(e));
}

int main(int argc, char** argv)
{
    // Optional: the prover executable whose embedded GPU code should be loaded onto each device.
    const char* host = nullptr;
    bool verbose = false;
    // ⚠ BEFORE ANY CUDA CALL, in parent and child alike: the runtime reads this when it first
    // initialises. EAGER makes it load every registered module when the context is created, which
    // is what turns "fails somewhere, later, at an unrelated line" into "fails here, now".
#ifdef _WIN32
    _putenv("CUDA_MODULE_LOADING=EAGER");
#else
    setenv("CUDA_MODULE_LOADING", "EAGER", 1);
#endif
    if (argc == 6 && !std::strcmp(argv[1], "--child"))
        return child(argv[2], std::atoi(argv[3]), std::atoi(argv[4]), std::atoi(argv[5]));
    for (int i = 1; i < argc; i++) {
        if (!std::strcmp(argv[i], "--verbose")) verbose = true;
        else host = argv[i];
    }
    std::vector<unsigned char> image;
    std::vector<std::pair<size_t, size_t>> fats;
    if (host) {
        if (!read_file(host, image)) {
            std::printf("could not read %s\n", host);
            return 1;
        }
        fats = find_fatbins(image);
        std::printf("%s: %zu bytes, %zu embedded CUDA fat binar%s\n\n", host, image.size(),
                    fats.size(), fats.size() == 1 ? "y" : "ies");
        // ⛔ ZERO IS NOT "NOTHING WAS REFUSED". A CPU-only build, the wrong file, or a format this
        // scan does not recognise all give zero, and reporting that as a clean result would be the
        // vacuous all-clear this project has been bitten by before.
        if (fats.empty()) {
            std::printf("NO GPU CODE FOUND in that file -- is it the CUDA build of the prover? "
                        "Nothing was tested.\n");
            return 1;
        }
    }

    int n = 0;
    cudaError_t e = cudaGetDeviceCount(&n);
    if (e != cudaSuccess) {
        std::printf("cudaGetDeviceCount failed: %s (%s)\n", cudaGetErrorName(e), cudaGetErrorString(e));
        return 1;
    }
    std::printf("CUDA devices: %d\n", n);

    int rt = 0, drv = 0;
    cudaRuntimeGetVersion(&rt);
    cudaDriverGetVersion(&drv);
    std::printf("runtime %d.%d, driver %d.%d\n\n", rt / 1000, (rt % 1000) / 10, drv / 1000, (drv % 1000) / 10);

    for (int id = 0; id < n; id++) {
        cudaDeviceProp p;
        if (cudaGetDeviceProperties(&p, id) != cudaSuccess) {
            std::printf("[%d] properties unreadable\n", id);
            continue;
        }
        std::printf("[%d] %s\n", id, p.name);
        std::printf("  compute capability               %d.%d\n", p.major, p.minor);
        std::printf("  total VRAM                       %.2f GB\n", (double)p.totalGlobalMem / (1024.0 * 1024.0 * 1024.0));
        std::printf("  SMs                              %d\n", p.multiProcessorCount);
        // ⚠ THIS PROPERTY IS THE SUSPECT. sppark's device filter accepts a card when this is
        // non-zero -- but the property is a statement about the DEVICE, and the launch can still be
        // refused by the PLATFORM (Windows WDDM). Printing both the claim and the outcome is the
        // only way to catch a card that advertises what the driver will not do.
        std::printf("  cooperativeLaunch (CLAIMED)      %d\n", p.cooperativeLaunch);
        std::printf("  computeMode                      %d\n", p.computeMode);

        if (cudaSetDevice(id) != cudaSuccess) {
            std::printf("  cudaSetDevice failed — skipping launch tests\n\n");
            continue;
        }

        int* d = nullptr;
        say("cudaMalloc", cudaMalloc(&d, sizeof(int)));

        // 1. ORDINARY LAUNCH. If this fails with cudaErrorNoKernelImageForDevice then the binary
        //    genuinely has no code for this card, and that IS the missing-sm_61 theory.
        touch<<<1, 32>>>(d);
        say("plain launch <<<1,32>>>", cudaGetLastError());
        say("  then synchronize", cudaDeviceSynchronize());

        // 2. COOPERATIVE LAUNCH, trivial kernel. Separates "cooperative launch is refused" from
        //    "the grid sync itself is the problem".
        void* args[] = { &d };
        say("cudaLaunchCooperativeKernel",
            cudaLaunchCooperativeKernel((const void*)touch, dim3(1), dim3(32), args, 0, 0));
        say("  then synchronize", cudaDeviceSynchronize());

        // 3. THE REAL SHAPE: a grid-wide sync, sized so it cannot fail merely for being too large.
        //    ⚠ A cooperative launch requires every block to be CO-RESIDENT, so the grid is capped
        //    at occupancy x SMs. Oversizing it yields cudaErrorCooperativeLaunchTooLarge, which
        //    would be a self-inflicted answer rather than a fact about the card.
        int per_sm = 0;
        say("occupancy query",
            cudaOccupancyMaxActiveBlocksPerMultiprocessor(&per_sm, touch_grid_sync, 32, 0));
        int blocks = per_sm * p.multiProcessorCount;
        if (blocks < 1) blocks = 1;
        std::printf("  co-resident blocks usable        %d (%d/SM x %d SMs)\n", blocks, per_sm, p.multiProcessorCount);
        say("coop launch + this_grid().sync()",
            cudaLaunchCooperativeKernel((const void*)touch_grid_sync, dim3(blocks), dim3(32), args, 0, 0));
        say("  then synchronize", cudaDeviceSynchronize());

        // 4. ⭐⭐ STREAM-ORDERED ALLOCATION. sppark calls cudaMallocAsync UNCONDITIONALLY
        //    (gpu_t.cuh:73 and :339) and never asks whether this device has memory pools. On a
        //    device where it does not, cudaMallocAsync returns cudaErrorNotSupported -- whose
        //    message is the bare words "operation not supported", which is what the prover reports.
        //    ⚠ And cudaGetLastError() is STICKY: a failure here would surface at the NEXT check,
        //    which in the NTT path is ntt.cuh:97 -- a line that has nothing to do with allocation.
        int pools = -1;
        say("query cudaDevAttrMemoryPoolsSupported",
            cudaDeviceGetAttribute(&pools, cudaDevAttrMemoryPoolsSupported, id));
        std::printf("  memoryPoolsSupported             %d\n", pools);
        int* da = nullptr;
        say("cudaMallocAsync (stream 0)", cudaMallocAsync(&da, sizeof(int), 0));
        say("cudaFreeAsync", cudaFreeAsync(da, 0));
        say("  then synchronize", cudaDeviceSynchronize());

        // 5. THE LAUNCH SHAPE THE NTT ACTUALLY USES: ntt.cuh:94 is
        //    `LDE_distribute_powers<<<stream.sm_count(), 1024, 0, stream>>>`. 1024 threads is the
        //    Pascal maximum, and a block that large can be refused for resources where 32 is fine.
        //    My first probe used 32 and therefore could not have caught this.
        touch<<<p.multiProcessorCount, 1024>>>(d);
        say("plain launch <<<SMs,1024>>>", cudaGetLastError());
        say("  then synchronize", cudaDeviceSynchronize());

        // 6. ⭐⭐ THE CALL THE PROVER ACTUALLY DIES ON: gpu_t.cuh:62 constructs four streams
        //    (stream_t zero + stream_t flipflop[3], all member initialisers, so they run BEFORE the
        //    gpu_t body) with exactly these flags, and reports "operation not supported".
        //    ⚠ stream_t never calls cudaSetDevice, so it builds on whatever device is current.
        cudaStream_t s1 = nullptr;
        say("cudaStreamCreateWithFlags(NonBlocking)",
            cudaStreamCreateWithFlags(&s1, cudaStreamNonBlocking));
        cudaStream_t s4[4] = { nullptr, nullptr, nullptr, nullptr };
        cudaError_t worst = cudaSuccess;
        for (int k = 0; k < 4; k++) {
            cudaError_t ek = cudaStreamCreateWithFlags(&s4[k], cudaStreamNonBlocking);
            if (ek != cudaSuccess && worst == cudaSuccess) worst = ek;
        }
        say("  x4, as gpu_t builds them", worst);
        for (int k = 0; k < 4; k++) if (s4[k]) cudaStreamDestroy(s4[k]);
        if (s1) cudaStreamDestroy(s1);

        cudaFree(d);

        // 7. ⭐⭐ THE PROVER'S OWN KERNELS, when a host executable was named.
        if (host) {
            std::printf("\n  -- the prover's own GPU code, loaded onto this device --\n");
            int refused = load_real_kernels(argv[0], host, image, fats, id, p.major, p.minor, verbose);
            if (refused > 0)
                std::printf("  => THIS DEVICE REFUSES PART OF THE PROVER'S GPU CODE. The lines above say which.\n");
            else if (refused == 0)
                std::printf("  => every module this method can judge loads on this device.\n");
            else
                std::printf("  => no module could be judged here; this says nothing about the card.\n");
        }
        std::printf("\n");
    }

    std::printf("done — a refusal above is an ANSWER, not a crash\n");
    return 0;
}
