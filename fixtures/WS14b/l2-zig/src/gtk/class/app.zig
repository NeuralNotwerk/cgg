const Surface = @import("surface.zig").Surface;
pub fn show(surface: *Surface) void {
    _ = surface.as(u8);
}
