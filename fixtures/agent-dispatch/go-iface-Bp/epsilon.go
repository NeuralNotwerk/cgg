package main

import "fmt"

type epsilon struct{}

func (x epsilon) process() { fmt.Println("Epsilon") }

func (x epsilon) unusedOp() { fmt.Println("u") }
