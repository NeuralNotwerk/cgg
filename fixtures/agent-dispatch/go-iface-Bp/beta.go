package main

import "fmt"

type beta struct{}

func (x beta) process() { fmt.Println("Beta") }

func (x beta) unusedOp() { fmt.Println("u") }
