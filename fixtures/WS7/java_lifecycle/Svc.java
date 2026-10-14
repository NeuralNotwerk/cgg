package demo;

import javax.annotation.PostConstruct;
import javax.annotation.PreDestroy;
import org.springframework.context.event.EventListener;
import org.springframework.transaction.event.TransactionalEventListener;
import org.springframework.scheduling.annotation.Scheduled;
import com.google.inject.Provides;
import com.google.inject.Inject;

class Svc {
    @PostConstruct
    private void init() { helper(); }

    @PreDestroy
    private void shutdown() {}

    @EventListener
    private void onEvent(Object e) {}

    @TransactionalEventListener
    private void onTx(Object e) {}

    @Scheduled(fixedRate = 10)
    private void tick() {}

    @Provides
    private String provideName() { return "x"; }

    @Inject
    private void setDeps(String s) {}

    private void helper() {}

    // near miss
    private void initLater() {}
}
