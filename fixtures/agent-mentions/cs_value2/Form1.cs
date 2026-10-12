using System;
using System.Windows.Forms;
using System.Runtime.Serialization;

namespace Ex
{
    public partial class Form1 : Form
    {
        private Button button1;
        private void InitializeComponent()
        {
            this.button1 = new Button();
            this.button1.Click += new EventHandler(this.button1_Click);
            this.Load += Form1_Load;
        }
        public Form1() { InitializeComponent(); }
        private void button1_Click(object sender, EventArgs e) { }
        private void Form1_Load(object sender, EventArgs e) { }

        [Obsolete("use other")]
        public void PublicWithAttrArgs() { }

        [Obsolete]
        public void PublicWithBareAttr() { }

        public void PublicNoAttr() { }

        [OnDeserialized]
        private void OnDeser(StreamingContext c) { }

        private void LocalFuncHost()
        {
            int Local(int x) => x;
            Func<int, int> f = Local;
        }
    }
}
