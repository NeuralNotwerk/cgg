const font = @import("../../main.zig");
const Canvas = @import("../canvas.zig").Canvas;

pub fn draw(canvas: *font.sprite.Canvas) void {
    canvas.rect();
}

pub fn drawDirect(canvas: *Canvas) void {
    canvas.rect();
}
