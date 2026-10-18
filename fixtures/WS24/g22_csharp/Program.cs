using Microsoft.AspNetCore.Builder;

var app = WebApplication.Create(args);
app.MapGet("/a", Handlers.List);
app.MapGet("/b", Local);
app.MapPost("/c", Handlers.Create);
app.Run();

static string Local() => "b";

public static class Handlers
{
    public static string List() => "a";
    public static string Create() => "c";
    public static string Unused() => "x";
}

public class Svc
{
    private readonly object _opts = new object();
    public void Register(WebApplication a)
    {
        // Near miss: a property read in argument position is no handler.
        a.MapGet("/d", () => _opts.ToString());
    }
}
