package org.x.streaming.checkpointing

import scala.reflect.ClassTag

class HDFSMetadataLog[T <: AnyRef : ClassTag](path: String) {
  def getLatest(): Option[(Long, T)] = None
  def get(batchId: Long): Option[T] = None
}
