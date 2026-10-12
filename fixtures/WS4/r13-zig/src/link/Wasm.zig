const std = @import("std");
const ml_dsa = @import("../../lib/std/ml_dsa.zig");

const GlobalImportId = struct {
    pub fn pack(x: u32) u32 {
        return x;
    }
    // A name this file declares is never another file's: not ml_dsa's pack.
    pub fn unpack(x: u32) u32 {
        return pack(x);
    }
};
const Other = struct {
    pub fn pack(x: u32) u32 {
        return x + 2;
    }
};

// Near miss: through the import, ml_dsa's pack is the call.
pub fn viaImport(x: u32) u32 {
    return ml_dsa.pack(x);
}
