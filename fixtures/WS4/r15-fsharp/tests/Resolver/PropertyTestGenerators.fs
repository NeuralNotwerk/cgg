module PropertyTestGenerators

type PackageTypes =
    /// A static member of a type in the caller's directory: never a bare call's target.
    static member PackageName (name: string) = name
