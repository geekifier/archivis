/** Centralized display metadata for providers and identifier types. */

interface DisplayEntry {
  label: string;
  colorClass: string;
}

const providers: Record<string, DisplayEntry> = {
  open_library: {
    label: 'Open Library',
    colorClass: 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400'
  },
  hardcover: {
    label: 'Hardcover',
    colorClass: 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400'
  },
  google_books: {
    label: 'Google Books',
    colorClass: 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
  },
  loc: {
    label: 'Library of Congress',
    colorClass: 'bg-orange-500/80 text-black'
  }
};

const identifierTypes: Record<string, DisplayEntry> = {
  isbn13: {
    label: 'ISBN-13',
    colorClass: 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400'
  },
  isbn10: {
    label: 'ISBN-10',
    colorClass: 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400'
  },
  asin: {
    label: 'ASIN',
    colorClass: 'bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-400'
  },
  google_books: {
    label: 'Google Books',
    colorClass: 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
  },
  open_library: {
    label: 'Open Library',
    colorClass: 'bg-indigo-100 text-indigo-800 dark:bg-indigo-900/30 dark:text-indigo-400'
  },
  hardcover: {
    label: 'Hardcover',
    colorClass: 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400'
  },
  lccn: {
    label: 'LCCN',
    colorClass: 'bg-orange-500/80 text-black'
  }
};

const topLevelSections: Record<string, string> = {
  server: 'Server',
  metadata: 'Metadata Providers',
  import: 'Import',
  auth: 'Authentication',
  isbn_scan: 'ISBN Scanning'
};

export function providerLabel(slug: string): string {
  return providers[slug]?.label ?? slug;
}

export function providerColorClass(slug: string): string {
  return providers[slug]?.colorClass ?? 'bg-muted text-muted-foreground';
}

export function identifierLabel(slug: string): string {
  return identifierTypes[slug]?.label ?? slug;
}

export function identifierColorClass(slug: string): string {
  return identifierTypes[slug]?.colorClass ?? 'bg-muted text-muted-foreground';
}

export function identifierTypeOptions(): { value: string; label: string }[] {
  return Object.entries(identifierTypes).map(([value, entry]) => ({
    value,
    label: entry.label
  }));
}

export function sectionLabel(section: string): string {
  if (topLevelSections[section]) return topLevelSections[section];
  if (section.startsWith('metadata.')) {
    const slug = section.slice('metadata.'.length);
    return providerLabel(slug);
  }
  return section;
}
