const std = @import("std");
const Surface = @import("surface.zig").Surface;

pub const SplitTree = struct {
    active: ?*Surface,

    fn getActiveSurface(self: *SplitTree) ?*Surface {
        return self.active;
    }

    fn getOne(self: *SplitTree) *Surface {
        return self.active.?;
    }

    pub fn closeActive(self: *SplitTree) void {
        const surface = self.getActiveSurface() orelse return;
        surface.close();
    }

    pub fn closeOne(self: *SplitTree) void {
        const s = self.getOne();
        s.close();
    }

    pub fn closeParam(surface: *Surface) void {
        surface.close();
    }
};
