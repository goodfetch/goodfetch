#include "lab_sqlite.h"
#include <node_api.h>
#include <stdlib.h>

typedef struct {lab_db *db;} box;
static void finalize(napi_env env,void *data,void *hint){(void)env;(void)hint;box *b=data;if(b->db)lab_close(b->db);free(b);}
static napi_value error(napi_env env,const char *message){napi_throw_error(env,NULL,message);return NULL;}
static char *string(napi_env env,napi_value value){size_t size=0;if(napi_get_value_string_utf8(env,value,NULL,0,&size)!=napi_ok||size>1000000)return NULL;char *out=malloc(size+1);if(!out)return NULL;if(napi_get_value_string_utf8(env,value,out,size+1,&size)!=napi_ok){free(out);return NULL;}return out;}
enum {OPEN,BEGIN,INSERT,COMMIT,ROLLBACK,INFO,CLOSE,QUERY,EXEC};
static napi_value call(napi_env env,napi_callback_info info){
    size_t argc=4;napi_value args[4];void *data=NULL;napi_get_cb_info(env,info,&argc,args,NULL,&data);int op=(int)(uintptr_t)data;napi_value result=NULL;
    if(op==OPEN){if(argc!=3)return error(env,"open expects path, format and timeout");char *path=string(env,args[0]),*format=string(env,args[1]);int32_t timeout=0;if(!path||!format||napi_get_value_int32(env,args[2],&timeout)!=napi_ok){free(path);free(format);return error(env,"invalid open arguments");}lab_db *db=lab_open(path,format,timeout);free(path);free(format);if(!db)return error(env,lab_error(NULL));box *b=malloc(sizeof(*b));if(!b){lab_close(db);return error(env,"allocation failed");}b->db=db;if(napi_create_external(env,b,finalize,NULL,&result)!=napi_ok){finalize(env,b,NULL);return error(env,"external handle failed");}return result;}
    box *b=NULL;if(argc>0&&napi_get_value_external(env,args[0],(void**)&b)!=napi_ok)return error(env,"invalid database handle");
    if(op==INFO){char *text=lab_info(b ? b->db : NULL);if(!text)return error(env,"SQLite metadata failed");napi_create_string_utf8(env,text,NAPI_AUTO_LENGTH,&result);lab_free(text);return result;}
    if(!b||!b->db)return error(env,"closed database handle");int rc=0;
    if(op==QUERY||op==EXEC){
        if(argc!=(op==QUERY?3:2))return error(env,"invalid SQL arguments");
        char *sql=string(env,args[1]),*parameters=op==QUERY?string(env,args[2]):NULL;
        if(!sql||(op==QUERY&&!parameters)){free(sql);free(parameters);return error(env,"invalid SQL strings");}
        if(op==QUERY){char *text=lab_query(b->db,sql,parameters);free(sql);free(parameters);if(!text)return error(env,lab_error(b->db));napi_create_string_utf8(env,text,NAPI_AUTO_LENGTH,&result);lab_free(text);return result;}
        rc=lab_exec(b->db,sql);free(sql);if(rc)return error(env,lab_error(b->db));napi_get_undefined(env,&result);return result;
    }
    if(op==INSERT){if(argc!=4)return error(env,"insert expects handle, id, value and payload");char *id=string(env,args[1]),*payload=string(env,args[3]);int64_t value=0;if(!id||!payload||napi_get_value_int64(env,args[2],&value)!=napi_ok){free(id);free(payload);return error(env,"invalid record");}rc=lab_insert(b->db,id,value,payload);free(id);free(payload);}
    else if(op==BEGIN)rc=lab_begin(b->db);else if(op==COMMIT)rc=lab_commit(b->db);else if(op==ROLLBACK)rc=lab_rollback(b->db);
    else if(op==CLOSE){rc=lab_close(b->db);b->db=NULL;if(rc)return error(env,"SQLite close failed");}
    else return error(env,"unknown native operation");
    if(rc)return error(env,lab_error(b->db));napi_get_undefined(env,&result);return result;
}
static napi_value init(napi_env env,napi_value exports){
    napi_property_descriptor methods[]={
        {"open",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)OPEN},
        {"begin",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)BEGIN},
        {"insert",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)INSERT},
        {"commit",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)COMMIT},
        {"rollback",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)ROLLBACK},
        {"info",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)INFO},
        {"close",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)CLOSE},
        {"query",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)QUERY},
        {"exec",NULL,call,NULL,NULL,NULL,napi_default,(void*)(uintptr_t)EXEC}
    };napi_define_properties(env,exports,sizeof(methods)/sizeof(methods[0]),methods);return exports;
}
NAPI_MODULE(goodfetch_lab_sqlite,init)
