/**
 * Generate a deterministic HSL color for an entity type name.
 * Uses a simple hash to distribute hues evenly across the color wheel.
 */
function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) - hash + str.charCodeAt(i)) | 0;
  }
  return Math.abs(hash);
}

export function entityColor(entityType: string): string {
  const hue = hashCode(entityType) % 360;
  return `hsl(${hue}, 65%, 55%)`;
}

export function entityColorDark(entityType: string): string {
  const hue = hashCode(entityType) % 360;
  return `hsl(${hue}, 55%, 45%)`;
}

export function entityColorBorder(entityType: string): string {
  const hue = hashCode(entityType) % 360;
  return `hsl(${hue}, 50%, 35%)`;
}
