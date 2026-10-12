package art

type Artifact struct{}

func NewDigestedArtifact() (*Artifact, error) { return &Artifact{}, nil }

func (a *Artifact) Digest() string { return "" }
