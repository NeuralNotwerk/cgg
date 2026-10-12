package main

import "fmt"

type delta struct{}

func (x delta) process() { fmt.Println("Delta") }

func (x delta) unusedOp() { fmt.Println("u") }
