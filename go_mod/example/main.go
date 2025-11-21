package main

import (
	"fmt"

	"github.com/gtsigner/never-jscore/go_mod/never_jscore"
)

func main() {
	fmt.Println("Creating context...")
	ctx, err := never_jscore.NewContext(true, true, -1)
	if err != nil {
		panic(err)
	}
	defer ctx.Close()

	fmt.Println("Executing JS (Exec)...")
	err = ctx.Exec("function add(a, b) { return a + b; }")
	if err != nil {
		panic(err)
	}

	fmt.Println("Compiling JS (Compile)...")
	err = ctx.Compile("function multiply(a, b) { return a * b; }")
	if err != nil {
		panic(err)
	}

	fmt.Println("Evaluating JS...")
	// Call added function
	res, err := ctx.Eval("add(1, 2)")
	if err != nil {
		panic(err)
	}
	fmt.Printf("Add Result: %s\n", res)

	// Test Call method
	fmt.Println("Testing Call method...")
	res, err = ctx.Call("add", 5, 7)
	if err != nil {
		panic(err)
	}
	fmt.Printf("Call(add, 5, 7) Result: %s\n", res)

	// Test GetStats
	fmt.Printf("Exec Count: %d\n", ctx.GetStats())

	// Test GC
	fmt.Println("Requesting GC...")
	ctx.GC()

	// Test Heap Statistics
	fmt.Println("Getting Heap Statistics...")
	stats, err := ctx.GetHeapStatistics()
	if err != nil {
		panic(err)
	}
	fmt.Printf("Heap Used: %d bytes\n", stats["used_heap_size"])

	// Test ResetStats
	fmt.Println("Resetting Stats...")
	ctx.ResetStats()
	fmt.Printf("Exec Count after reset: %d\n", ctx.GetStats())

	// Call compiled function
	res, err = ctx.Eval("multiply(3, 4)")
	if err != nil {
		panic(err)
	}
	fmt.Printf("Multiply Result: %s\n", res)

	// Test console.log
	fmt.Println("Testing console.log (check stdout)...")
	ctx.Exec("console.log('Hello from JS in Go!');")
}
