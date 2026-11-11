/* peak-gap (measurement only): routes the C code's malloc family to mimalloc,
 * what libmimalloc-sys's `override` feature does (src/alloc-override.c), as an
 * extra object in the re-link. Zig's libc defines these weak, so these win. */
#include <stddef.h>
void *mi_malloc(size_t); void *mi_calloc(size_t, size_t); void *mi_realloc(void *, size_t);
void mi_free(void *); void *mi_malloc_aligned(size_t, size_t); size_t mi_usable_size(const void *);
int mi_posix_memalign(void **, size_t, size_t); void *mi_reallocarray(void *, size_t, size_t);
void *mi_valloc(size_t); void *mi_pvalloc(size_t); void *mi_memalign(size_t, size_t);
void *mi_aligned_alloc(size_t, size_t);
void *malloc(size_t n) { return mi_malloc(n); }
void *calloc(size_t n, size_t s) { return mi_calloc(n, s); }
void *realloc(void *p, size_t n) { return mi_realloc(p, n); }
void free(void *p) { mi_free(p); }
void *aligned_alloc(size_t a, size_t n) { return mi_aligned_alloc(a, n); }
void *memalign(size_t a, size_t n) { return mi_memalign(a, n); }
void *valloc(size_t n) { return mi_valloc(n); }
void *pvalloc(size_t n) { return mi_pvalloc(n); }
int posix_memalign(void **p, size_t a, size_t n) { return mi_posix_memalign(p, a, n); }
void *reallocarray(void *p, size_t n, size_t s) { return mi_reallocarray(p, n, s); }
size_t malloc_usable_size(void *p) { return mi_usable_size(p); }
