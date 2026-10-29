// No update is ever found in the preview; the update-check event can still be
// played (driver.ts) to show the banner.
export async function check(): Promise<null> {
  return null;
}
