import { redirect, useRouter } from "next/navigation";
import dayjs from "@scope/dayjs";
import { redirect as go } from "@/lib/navigation";

export function page() {
  // `next/navigation` is a dependency: external, never src/lib/navigation.ts.
  redirect("/");
  useRouter();
  // A workspace package re-exporting a dependency: labelled with it.
  dayjs();
  // Near miss: a path alias still reaches the tree's file.
  go("/home");
}
