#define _GNU_SOURCE
#include "lab_sqlite.h"
#include "sqlite3.h"
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct lab_db { sqlite3 *db; sqlite3_stmt *insert; };
static char open_error[512];
const char *lab_error(lab_db *handle) { return handle ? sqlite3_errmsg(handle->db) : open_error; }
lab_db *lab_open(const char *path, const char *format, int busy_ms) {
    if (!path || !format || busy_ms < 0 || busy_ms > 60000 ||
        (strcmp(format,"sqlite-wal-full") && strcmp(format,"sqlite-wal-normal") && strcmp(format,"sqlite-delete-full"))) {
        snprintf(open_error,sizeof(open_error),"unsupported SQLite contract"); return NULL;
    }
    lab_db *h=calloc(1,sizeof(*h)); if (!h) return NULL;
    int rc=sqlite3_open_v2(path,&h->db,SQLITE_OPEN_READWRITE|SQLITE_OPEN_CREATE|SQLITE_OPEN_FULLMUTEX,NULL);
    if (rc==SQLITE_OK) rc=sqlite3_busy_timeout(h->db,busy_ms);
    if (rc==SQLITE_OK) rc=sqlite3_exec(h->db,strstr(format,"wal") ? "PRAGMA journal_mode=WAL" : "PRAGMA journal_mode=DELETE",NULL,NULL,NULL);
    if (rc==SQLITE_OK) rc=sqlite3_exec(h->db,strstr(format,"normal") ? "PRAGMA synchronous=NORMAL" : "PRAGMA synchronous=FULL",NULL,NULL,NULL);
    if (rc==SQLITE_OK) rc=sqlite3_exec(h->db,"PRAGMA wal_autocheckpoint=1000;CREATE TABLE IF NOT EXISTS records(id TEXT PRIMARY KEY,value INTEGER NOT NULL,payload TEXT NOT NULL);",NULL,NULL,NULL);
    if (rc==SQLITE_OK) rc=sqlite3_prepare_v2(h->db,"INSERT INTO records(id,value,payload) VALUES(?,?,?)",-1,&h->insert,NULL);
    if (rc!=SQLITE_OK) { snprintf(open_error,sizeof(open_error),"%s",sqlite3_errmsg(h->db)); if(h->insert)sqlite3_finalize(h->insert); sqlite3_close_v2(h->db); free(h);return NULL; }
    return h;
}
int lab_begin(lab_db *h) { return h ? sqlite3_exec(h->db,"BEGIN IMMEDIATE",NULL,NULL,NULL) : SQLITE_MISUSE; }
int lab_insert(lab_db *h,const char *id,int64_t value,const char *payload) {
    if(!h||!id||!payload)return SQLITE_MISUSE;
    sqlite3_reset(h->insert);sqlite3_clear_bindings(h->insert);
    int rc=sqlite3_bind_text(h->insert,1,id,-1,SQLITE_TRANSIENT);
    if(rc==SQLITE_OK)rc=sqlite3_bind_int64(h->insert,2,value);
    if(rc==SQLITE_OK)rc=sqlite3_bind_text(h->insert,3,payload,-1,SQLITE_TRANSIENT);
    if(rc==SQLITE_OK)rc=sqlite3_step(h->insert);
    return rc==SQLITE_DONE ? SQLITE_OK : rc;
}
int lab_commit(lab_db *h) { return h ? sqlite3_exec(h->db,"COMMIT",NULL,NULL,NULL) : SQLITE_MISUSE; }
int lab_rollback(lab_db *h) { return h ? sqlite3_exec(h->db,"ROLLBACK",NULL,NULL,NULL) : SQLITE_MISUSE; }
int lab_close(lab_db *h) { if(!h)return SQLITE_OK;sqlite3_finalize(h->insert);int rc=sqlite3_close_v2(h->db);free(h);return rc; }
void lab_free(void *p) { sqlite3_free(p); }
char *lab_info(lab_db *handle) {
    lab_db *h=handle ? handle : lab_open(":memory:","sqlite-wal-full",5000);if(!h)return NULL;
    Dl_info image={0};dladdr((void*)&lab_open,&image);
    sqlite3_stmt *wal=NULL;int checkpoint=-1;
    if(sqlite3_prepare_v2(h->db,"PRAGMA wal_autocheckpoint",-1,&wal,NULL)==SQLITE_OK&&sqlite3_step(wal)==SQLITE_ROW)checkpoint=sqlite3_column_int(wal,0);
    if(wal)sqlite3_finalize(wal);
    const char *sql="SELECT json_object('sqlite',sqlite_version(),'sourceId',sqlite_source_id(),'libraryPath',?,'compileOptions',(SELECT json_group_array(compile_options) FROM pragma_compile_options),'journal_mode',(SELECT journal_mode FROM pragma_journal_mode),'synchronous',(SELECT synchronous FROM pragma_synchronous),'busy_timeout',(SELECT timeout FROM pragma_busy_timeout),'wal_autocheckpoint',?,'page_size',(SELECT page_size FROM pragma_page_size))";
    sqlite3_stmt *s=NULL;char *result=NULL;
    if(sqlite3_prepare_v2(h->db,sql,-1,&s,NULL)==SQLITE_OK){sqlite3_bind_text(s,1,image.dli_fname ? image.dli_fname : "unknown",-1,SQLITE_TRANSIENT);sqlite3_bind_int(s,2,checkpoint);if(sqlite3_step(s)==SQLITE_ROW)result=sqlite3_mprintf("%s",sqlite3_column_text(s,0));}
    if(s)sqlite3_finalize(s);if(!handle)lab_close(h);return result;
}
