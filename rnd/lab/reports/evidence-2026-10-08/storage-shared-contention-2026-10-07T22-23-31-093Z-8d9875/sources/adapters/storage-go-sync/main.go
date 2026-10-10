package main

/*
#cgo CFLAGS: -I${SRCDIR}/../sqlite-common
#include <stdlib.h>
#include "lab_sqlite.h"
*/
import "C"
import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"strings"
	"time"
	"unsafe"
)

type Request struct {
	Version int             `json:"version"`
	ID      int             `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
}
type Prepare struct {
	Path   string `json:"path"`
	Format string `json:"format"`
	Busy   int    `json:"busyTimeoutMs"`
}
type Batch struct {
	ID           string `json:"batchId"`
	Seed         int64  `json:"seed"`
	Rows         int    `json:"rows"`
	PayloadBytes int    `json:"payloadBytes"`
}
type Record struct {
	ID      string `json:"id"`
	Value   int64  `json:"value"`
	Payload string `json:"payload"`
}

func dbError(db *C.lab_db) error { return fmt.Errorf("%s", C.GoString(C.lab_error(db))) }
func info(db *C.lab_db) (any, error) {
	text := C.lab_info(db)
	if text == nil {
		return nil, fmt.Errorf("SQLite metadata failed")
	}
	defer C.lab_free(unsafe.Pointer(text))
	var result map[string]any
	if err := json.Unmarshal([]byte(C.GoString(text)), &result); err != nil {
		return nil, err
	}
	result["backend"] = "storage-go-sync"
	result["runtime"] = runtime.Version()
	result["binding"] = "synchronous cgo; one common C call per inserted row"
	return result, nil
}
func writeBatch(db *C.lab_db, b Batch) (any, error) {
	if db == nil {
		return nil, fmt.Errorf("not prepared")
	}
	if b.Rows < 1 || b.Rows > 10000 || b.PayloadBytes < 0 || b.PayloadBytes > 65536 || strings.ContainsRune(b.ID, 0) {
		return nil, fmt.Errorf("invalid batch")
	}
	began := time.Now()
	payload := strings.Repeat("x", b.PayloadBytes)
	rows := make([]Record, b.Rows)
	var encoded strings.Builder
	for i := range rows {
		rows[i] = Record{fmt.Sprintf("%s:%d", b.ID, i), b.Seed + int64(i), payload}
		line, err := json.Marshal(rows[i])
		if err != nil {
			return nil, err
		}
		encoded.Write(line)
		encoded.WriteByte('\n')
	}
	serializationMS := float64(time.Since(began).Nanoseconds()) / 1e6
	payloadC := C.CString(payload)
	defer C.free(unsafe.Pointer(payloadC))
	if C.lab_begin(db) != 0 {
		return nil, dbError(db)
	}
	for _, r := range rows {
		id := C.CString(r.ID)
		code := C.lab_insert(db, id, C.int64_t(r.Value), payloadC)
		C.free(unsafe.Pointer(id))
		if code != 0 {
			err := dbError(db)
			C.lab_rollback(db)
			return nil, err
		}
	}
	if C.lab_commit(db) != 0 {
		err := dbError(db)
		C.lab_rollback(db)
		return nil, err
	}
	return map[string]any{"status": "ok", "rows": b.Rows, "metrics": map[string]any{"serializationMs": serializationMS, "writerAckMs": float64(time.Since(began).Nanoseconds()) / 1e6, "encodedBytes": encoded.Len()}}, nil
}
func main() {
	var db *C.lab_db
	defer func() {
		if db != nil {
			C.lab_close(db)
		}
	}()
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), 128000)
	out := bufio.NewWriter(os.Stdout)
	for scanner.Scan() {
		var m Request
		if json.Unmarshal(scanner.Bytes(), &m) != nil || m.Version != 1 || m.ID < 1 {
			break
		}
		var result any
		var err error
		switch m.Method {
		case "hello":
			result, err = info(db)
		case "prepare":
			var p Prepare
			if db != nil {
				err = fmt.Errorf("already prepared")
			} else if err = json.Unmarshal(m.Params, &p); err == nil {
				path, format := C.CString(p.Path), C.CString(p.Format)
				db = C.lab_open(path, format, C.int(p.Busy))
				C.free(unsafe.Pointer(path))
				C.free(unsafe.Pointer(format))
				if db == nil {
					err = dbError(nil)
				} else {
					result = map[string]bool{"ready": true}
				}
			}
		case "run":
			var b Batch
			if err = json.Unmarshal(m.Params, &b); err == nil {
				result, err = writeBatch(db, b)
			}
		case "close":
			if db != nil {
				if C.lab_close(db) != 0 {
					err = fmt.Errorf("SQLite close failed")
				}
				db = nil
			}
			result = map[string]bool{"closed": true}
		default:
			err = fmt.Errorf("unknown method")
		}
		response := map[string]any{"version": 1, "id": m.ID}
		if err != nil {
			response["error"] = err.Error()
		} else {
			response["result"] = result
		}
		json.NewEncoder(out).Encode(response)
		out.Flush()
		if m.Method == "close" {
			break
		}
	}
}
