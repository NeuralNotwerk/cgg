package com.acme.j;
class JBase { void start() { } }
class JMid extends JBase { }
class JLeaf extends JMid { @Override void start() { super.start(); } }
