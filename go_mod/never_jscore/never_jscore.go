package never_jscore

/*
#cgo LDFLAGS: -L../../target/release -lnever_jscore
#include <stdlib.h>

typedef void* ContextPtr;

void never_jscore_init();
ContextPtr never_jscore_new(int enable_extensions, int enable_logging, long long random_seed);
void never_jscore_free(ContextPtr ptr);
char* never_jscore_eval(ContextPtr ptr, const char* code);
void never_jscore_free_string(char* s);
int never_jscore_exec(ContextPtr ptr, const char* code);
int never_jscore_compile(ContextPtr ptr, const char* code);
*/
import "C"
import (
	"errors"
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
// This is an alias for Exec, provided for API compatibility
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
