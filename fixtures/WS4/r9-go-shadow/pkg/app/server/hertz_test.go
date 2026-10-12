package server

import (
	"testing"

	"example.com/hz/pkg/app"
	c "example.com/hz/pkg/app/client"
)

// The parameter `c` shadows the import alias `c`: c.JSON is app.RequestContext.JSON.
func handler(ctx int, c *app.RequestContext) {
	c.JSON(200)
}

// An untyped local shadows it too: not refused as a miss in package client.
func untyped(t *testing.T) {
	c := struct{ JSON func(int) }{}
	c.JSON(1)
}

// Near miss: here `c` is the package, and package client has no JSON.
func viaPackage(t *testing.T) {
	c.Do()
	c.JSON(2)
}
