const std = @import("std");
const link = @import("link.zig");
// A file-local alias of link.zig's function: never std.Build's copy.
const addRunArtifact = link.addRunArtifact;

pub fn testMacho(x: u32) u32 {
    return addRunArtifact(x);
}
