package main

func target() int { return 1 }
func live() int   { return 2 }

func main() {
	live()
	return
	target()
}
