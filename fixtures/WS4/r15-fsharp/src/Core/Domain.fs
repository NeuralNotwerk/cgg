module Paket.Domain

/// The function a bare `PackageName "x"` names.
let PackageName (name: string) = name.ToLowerInvariant()
