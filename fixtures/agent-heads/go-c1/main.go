package main

import (
	"encoding/json"
	"github.com/google/uuid"
	"github.com/pkg/errors"
	"go.uber.org/zap"
)

func run(err error) error {
	_ = uuid.NewString()
	_ = uuid.MustParse("x")
	zap.S().Infow("hi")
	_, _ = json.Marshal(1)
	return errors.Wrap(err, "x")
}

func main() { _ = run(nil) }
