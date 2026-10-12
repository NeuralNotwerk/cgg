const link = @import("link.zig");
const addRunArtifact = link.addRunArtifact;

pub fn testMacho(x: u32) u32 {
    return addRunArtifact(x);
}
