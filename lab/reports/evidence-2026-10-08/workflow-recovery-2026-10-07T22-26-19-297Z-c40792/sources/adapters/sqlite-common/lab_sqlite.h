#ifndef AUTOMAN_LAB_SQLITE_H
#define AUTOMAN_LAB_SQLITE_H
#include <stdint.h>
#define LAB_PUBLIC __attribute__((visibility("default")))
typedef struct lab_db lab_db;
LAB_PUBLIC lab_db *lab_open(const char *path, const char *format, int busy_ms);
LAB_PUBLIC int lab_begin(lab_db *db);
LAB_PUBLIC int lab_insert(lab_db *db, const char *id, int64_t value, const char *payload);
LAB_PUBLIC int lab_commit(lab_db *db);
LAB_PUBLIC int lab_rollback(lab_db *db);
LAB_PUBLIC int lab_close(lab_db *db);
LAB_PUBLIC const char *lab_error(lab_db *db);
LAB_PUBLIC char *lab_info(lab_db *db);
LAB_PUBLIC void lab_free(void *ptr);
#endif
