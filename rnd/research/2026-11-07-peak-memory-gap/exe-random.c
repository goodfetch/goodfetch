/* peak-gap (measurement only): at start-up, madvise(MADV_RANDOM) on the
 * executable's own read-only PT_LOAD segments (code and read-only data), so a
 * page fault there reads only that page instead of a read-around window, and
 * fault-around finds no cached neighbours to map. PEAKGAP_EXE_RANDOM=0 skips it. */
#define _GNU_SOURCE
#include <link.h>
#include <stdlib.h>
#include <sys/mman.h>

static int one(struct dl_phdr_info *info, size_t size, void *done) {
    (void)size;
    if (*(int *)done) return 1;
    *(int *)done = 1;
    for (int i = 0; i < info->dlpi_phnum; i++) {
        const ElfW(Phdr) *p = &info->dlpi_phdr[i];
        if (p->p_type != PT_LOAD || (p->p_flags & PF_W)) continue;
        unsigned long lo = (info->dlpi_addr + p->p_vaddr) & ~4095UL;
        unsigned long hi = (info->dlpi_addr + p->p_vaddr + p->p_memsz + 4095) & ~4095UL;
        madvise((void *)lo, hi - lo, MADV_RANDOM);
    }
    return 1;
}

__attribute__((constructor(102))) static void peakgap_exe_random(void) {
    const char *v = getenv("PEAKGAP_EXE_RANDOM");
    if (v && v[0] == '0') return;
    int done = 0;
    dl_iterate_phdr(one, &done);
}
