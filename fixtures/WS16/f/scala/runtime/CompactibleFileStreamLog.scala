package org.x.streaming.runtime

import scala.reflect.ClassTag
import org.x.streaming.checkpointing.HDFSMetadataLog

abstract class CompactibleFileStreamLog[T <: AnyRef : ClassTag](
    version: Int,
    path: String)
  extends HDFSMetadataLog[Array[T]](path) {
  def compactInterval: Int = 10
}
