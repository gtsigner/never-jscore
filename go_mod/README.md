# 封装Go实现

```
go get github.com/gtsigner/never-jscore/go_mod
```

func main() {
	fmt.Println("Creating context...")
	ctx, err := never_jscore.NewContext(true, true, -1)
	if err != nil {
		panic(err)
	}
	defer ctx.Close()
}    
```
