package org.x.streaming.runtime

class FileStreamSourceLog(
    version: Int,
    path: String)
  extends CompactibleFileStreamLog[String](version, path) {

  def latestId(endId: Option[Long]): Long = {
    val endBatchId = endId.orElse(getLatest().map(_._1)).getOrElse(0L)
    endBatchId
  }
}
