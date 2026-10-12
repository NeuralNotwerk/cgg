package app

type RequestContext struct{}

// JSON is the method a handler's `c.JSON(...)` reaches.
func (c *RequestContext) JSON(code int) {}
