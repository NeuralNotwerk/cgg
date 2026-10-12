package main

import "example.com/r1/art"

func main() {
	if true {
		art, err := art.NewDigestedArtifact()
		_ = err
		art.Digest()
	}
	if true {
		a, _ := art.NewDigestedArtifact()
		a.Digest()
	}
}
