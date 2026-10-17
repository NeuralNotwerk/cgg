package app

import akka.actor.Actor

class MyActor extends Actor {
  override def hookSc(): Unit = {}
  private def unusedSc(): Unit = {}
}
