class Tool { public void Work() {} }
class Other { public void Work() {} }
class Factory {
    public static Tool Make() { return new Tool(); }
    Other Oth() => new Other();
    async System.Threading.Tasks.Task<Tool> MakeAsync() { return new Tool(); }
    void Use() {
        var t = Make();
        t.Work();
        Make().Work();
        var o = Oth();
        o.Work();
    }
    void A() { var x = Make(); x.Work(); }
    void B() { var x = Oth(); x.Work(); }
}
