/** Facts about this build shown by the skeleton pages. */
export const SLICE = 'V1-0';

/** One line describing what this scaffold is and is not. */
export function scaffoldNotice(page: string): string {
  return `${page}: HealthierYou ${SLICE} scaffold. The real ${page.toLowerCase()} arrives in a later slice.`;
}
