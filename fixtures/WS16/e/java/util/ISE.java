package util;
public class ISE extends RuntimeException {
  public ISE(String formatText, Object... arguments) {
    super(formatText);
  }
  public ISE(Throwable cause, String formatText, Object... arguments) {
    super(formatText, cause);
  }
}
