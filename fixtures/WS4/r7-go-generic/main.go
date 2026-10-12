package main

import "example.com/r7/ordered"

func main() {
	m := ordered.NewMap[string, int]()
	m.ReplaceKey("a", "b")
}
