package svc

import (
	"example.com/r3/idv"
	"github.com/ext/oss"
)

type Local struct{}

func (l *Local) Endpoint() string { return "" }

func NewLocal() *Local             { return &Local{} }
func GetIdv() idv.Provider         { return &idv.Real{} }
func GetStore() (oss.Store, error) { return nil, nil }
func Untyped()                     {}
