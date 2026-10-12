package main

import "example.com/r2/ttl"

func main() {
	ttl.CacheDefault.Int()
	ttl.Seconds()
	ttl.Missing()
}
