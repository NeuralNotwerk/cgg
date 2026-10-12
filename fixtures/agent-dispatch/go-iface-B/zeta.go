package main

import "fmt"

type zeta struct{}

func (x zeta) process() { fmt.Println("Zeta") }

func (x zeta) unusedOp() { fmt.Println("u") }
