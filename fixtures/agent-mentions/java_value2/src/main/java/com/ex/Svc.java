package com.ex;

import java.io.*;
import java.util.*;
import java.util.function.*;
import javax.annotation.PostConstruct;
import org.springframework.scheduling.annotation.Scheduled;
import org.springframework.context.event.EventListener;

public class Svc implements Serializable {
    private void readObject(ObjectInputStream in) throws IOException, ClassNotFoundException { }
    private void writeObject(ObjectOutputStream out) throws IOException { }
    private Object readResolve() { return this; }
    @PostConstruct
    private void init() { }
    @Scheduled(fixedRate = 1000)
    private void tick() { }
    @EventListener
    private void onEvent(Object e) { }
    private String keyOf(String s) { return s; }
    private static Svc create() { return new Svc(); }
    private void anonTarget() { }
    private void deadControl() { }

    public void use(List<String> xs) {
        xs.sort(Comparator.comparing(this::keyOf));
        Supplier<Svc> s = Svc::create;
        Runnable r = new Runnable() { public void run() { anonTarget(); } };
        r.run();
    }
}
