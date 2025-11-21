package never_jscore

/*
#cgo darwin,arm64  LDFLAGS: -L${SRCDIR}/lib/aarch64-apple-darwin -lnever_jscore -framework Security -framework SystemConfiguration -framework CoreFoundation
#cgo darwin,amd64  LDFLAGS: -L${SRCDIR}/lib/x86_64-apple-darwin -lnever_jscore -framework Security -framework SystemConfiguration -framework CoreFoundation
#cgo linux,amd64  LDFLAGS: -L${SRCDIR}/lib/x86_64-unknown-linux-gnu -lnever_jscore -lrt -lpthread -ldl
#cgo linux,arm64  LDFLAGS: -L${SRCDIR}/lib/aarch64-unknown-linux-gnu -lnever_jscore -lrt -lpthread -ldl
#cgo windows,amd64 LDFLAGS: -L${SRCDIR}/lib/x86_64-pc-windows-msvc -lnever_jscore
#include <stdlib.h>

typedef void* ContextPtr;

void never_jscore_init();
ContextPtr never_jscore_new(int enable_extensions, int enable_logging, long long random_seed);
void never_jscore_free(ContextPtr ptr);
char* never_jscore_eval(ContextPtr ptr, const char* code);
void never_jscore_free_string(char* s);
int never_jscore_exec(ContextPtr ptr, const char* code);
int never_jscore_compile(ContextPtr ptr, const char* code);
void never_jscore_gc(ContextPtr ptr);
size_t never_jscore_get_stats(ContextPtr ptr);
void never_jscore_reset_stats(ContextPtr ptr);
char* never_jscore_get_heap_statistics(ContextPtr ptr);
int never_jscore_take_heap_snapshot(ContextPtr ptr, const char* file_path);
*/
import "C"
import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"unsafe"
)

func init() {
	C.never_jscore_init()
}

type Context struct {
	ptr C.ContextPtr
}

func NewContext(enableExtensions bool, enableLogging bool, randomSeed int64) (*Context, error) {
	cExt := C.int(0)
	if enableExtensions {
		cExt = 1
	}
	cLog := C.int(0)
	if enableLogging {
		cLog = 1
	}

	ptr := C.never_jscore_new(cExt, cLog, C.longlong(randomSeed))
	if ptr == nil {
		return nil, errors.New("failed to create context")
	}
	return &Context{ptr: ptr}, nil
}

func (c *Context) Close() {
	if c.ptr != nil {
		C.never_jscore_free(c.ptr)
		c.ptr = nil
	}
}

// Eval executes code and returns JSON result string
func (c *Context) Eval(code string) (string, error) {
	if c.ptr == nil {
		return "", errors.New("context closed")
	}

	cCode := C.CString(code)
	defer C.free(unsafe.Pointer(cCode))

	cResult := C.never_jscore_eval(c.ptr, cCode)
	if cResult == nil {
		return "", errors.New("execution failed")
	}
	defer C.never_jscore_free_string(cResult)

	return C.GoString(cResult), nil
}

// Exec executes code without returning result (side effects only)
func (c *Context) Exec(code string) error {
	if c.ptr == nil {
		return errors.New("context closed")
	}

	cCode := C.CString(code)
	defer C.free(unsafe.Pointer(cCode))

	ret := C.never_jscore_exec(c.ptr, cCode)
	if ret != 0 {
		return errors.New("execution failed")
	}
	return nil
}

// Compile compiles code and loads it into the global scope
func (c *Context) Compile(code string) error {
	if c.ptr == nil {
		return errors.New("context closed")
	}

	cCode := C.CString(code)
	defer C.free(unsafe.Pointer(cCode))

	ret := C.never_jscore_compile(c.ptr, cCode)
	if ret != 0 {
		return errors.New("compilation failed")
	}
	return nil
}

// Call calls a JavaScript function with arguments
func (c *Context) Call(name string, args ...interface{}) (string, error) {
	// Serialize arguments to JSON
	jsonArgs := make([]string, len(args))
	for i, arg := range args {
		b, err := json.Marshal(arg)
		if err != nil {
			return "", fmt.Errorf("failed to marshal argument %d: %v", i, err)
		}
		jsonArgs[i] = string(b)
	}

	// Construct function call string
	callCode := fmt.Sprintf("%s(%s)", name, strings.Join(jsonArgs, ", "))

	// Execute
	return c.Eval(callCode)
}

// GC requests garbage collection
func (c *Context) GC() {
	if c.ptr != nil {
		C.never_jscore_gc(c.ptr)
	}
}

// GetStats returns execution statistics (execution count)
func (c *Context) GetStats() uint64 {
	if c.ptr != nil {
		return uint64(C.never_jscore_get_stats(c.ptr))
	}
	return 0
}

// ResetStats resets execution statistics
func (c *Context) ResetStats() {
	if c.ptr != nil {
		C.never_jscore_reset_stats(c.ptr)
	}
}

// GetHeapStatistics returns V8 heap statistics
func (c *Context) GetHeapStatistics() (map[string]uint64, error) {
	if c.ptr == nil {
		return nil, errors.New("context closed")
	}

	cJson := C.never_jscore_get_heap_statistics(c.ptr)
	if cJson == nil {
		return nil, errors.New("failed to get heap statistics")
	}
	defer C.never_jscore_free_string(cJson)

	jsonStr := C.GoString(cJson)
	var stats map[string]uint64
	if err := json.Unmarshal([]byte(jsonStr), &stats); err != nil {
		return nil, fmt.Errorf("failed to unmarshal heap stats: %v", err)
	}
	return stats, nil
}

// TakeHeapSnapshot takes a heap snapshot and saves it to the specified file
func (c *Context) TakeHeapSnapshot(filePath string) error {
	if c.ptr == nil {
		return errors.New("context closed")
	}

	cPath := C.CString(filePath)
	defer C.free(unsafe.Pointer(cPath))

	ret := C.never_jscore_take_heap_snapshot(c.ptr, cPath)
	if ret != 0 {
		return errors.New("failed to take heap snapshot")
	}
	return nil
}
