package app

import org.springframework.scheduling.annotation.Scheduled
import org.springframework.context.annotation.Bean
import org.springframework.context.event.EventListener
import org.springframework.kafka.annotation.KafkaListener
import javax.annotation.PostConstruct
import javax.inject.Inject

class Jobs {
    @Scheduled(fixedRate = 1000)
    fun tick() { helperTick() }

    private fun helperTick() {}

    @KafkaListener(topics = ["orders"])
    fun onOrder(msg: String) {}

    @Bean
    fun clock(): Any = Any()

    @EventListener
    fun onStart(e: Any) {}

    @PostConstruct
    fun init() {}

    @Inject
    fun setDep(d: Any) {}

    fun neverCalled() {}
}
