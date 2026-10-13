namespace Samples
{
    public partial class Form1
    {
        public Form1() { InitializeComponent(); }
        public void Complete(int x) {}
        public void Complete(string x) {}
        public void Run() { Complete(1); }
    }
}
