import { CalendarEventBuilder } from './builder';
export function go() {
  new CalendarEventBuilder({}).withTeam(1).build();
  new CalendarEventBuilder({}).withTeam(1);
}
