import type {
  BookFormat,
  BookListParams,
  LibraryFilterState,
  MetadataStatus,
  ResolutionOutcome,
  ResolutionState,
  TagMatchMode
} from '$lib/api/types.js';

export interface RelationFilter {
  id: string;
  name: string;
}

export interface TagFilter {
  id: string;
  name: string;
  category: string | null;
}

export type IdentifierType = string;

function createFilterStore() {
  // --- Basic filters ---
  let activeFormat = $state<BookFormat | null>(null);
  let activeStatus = $state<MetadataStatus | null>(null);

  // --- Relation filters ---
  let activeAuthor = $state<RelationFilter | null>(null);
  let activeSeries = $state<RelationFilter | null>(null);
  let activePublisher = $state<RelationFilter | null>(null);

  // --- Tag filters ---
  let activeTags = $state<TagFilter[]>([]);
  let activeTagMatch = $state<TagMatchMode>('any');

  // --- State filters ---
  let activeTrusted = $state<boolean | null>(null);
  let activeLocked = $state<boolean | null>(null);
  let activeResolutionState = $state<ResolutionState | null>(null);
  let activeResolutionOutcome = $state<ResolutionOutcome | null>(null);

  // --- Content filters ---
  let activeLanguage = $state<string | null>(null);
  let activeYearMin = $state<number | null>(null);
  let activeYearMax = $state<number | null>(null);

  // --- Presence filters ---
  let activeHasCover = $state<boolean | null>(null);
  let activeHasDescription = $state<boolean | null>(null);
  let activeHasIdentifiers = $state<boolean | null>(null);

  // --- Identifier lookup ---
  let activeIdentifierType = $state<IdentifierType | null>(null);
  let activeIdentifierValue = $state<string>('');

  const hasActiveFilters = $derived(
    activeFormat !== null ||
      activeStatus !== null ||
      activeAuthor !== null ||
      activeSeries !== null ||
      activePublisher !== null ||
      activeTags.length > 0 ||
      activeTrusted !== null ||
      activeLocked !== null ||
      activeResolutionState !== null ||
      activeResolutionOutcome !== null ||
      activeLanguage !== null ||
      activeYearMin !== null ||
      activeYearMax !== null ||
      activeHasCover !== null ||
      activeHasDescription !== null ||
      activeHasIdentifiers !== null ||
      activeIdentifierValue.trim() !== ''
  );

  const activeFilterCount = $derived(
    (activeFormat !== null ? 1 : 0) +
      (activeStatus !== null ? 1 : 0) +
      (activeAuthor !== null ? 1 : 0) +
      (activeSeries !== null ? 1 : 0) +
      (activePublisher !== null ? 1 : 0) +
      (activeTags.length > 0 ? 1 : 0) +
      (activeTrusted !== null ? 1 : 0) +
      (activeLocked !== null ? 1 : 0) +
      (activeResolutionState !== null ? 1 : 0) +
      (activeResolutionOutcome !== null ? 1 : 0) +
      (activeLanguage !== null ? 1 : 0) +
      (activeYearMin !== null || activeYearMax !== null ? 1 : 0) +
      (activeHasCover !== null ? 1 : 0) +
      (activeHasDescription !== null ? 1 : 0) +
      (activeHasIdentifiers !== null ? 1 : 0) +
      (activeIdentifierValue.trim() !== '' ? 1 : 0)
  );

  // --- Setters ---

  function setFormat(format: BookFormat | null) {
    activeFormat = activeFormat === format ? null : format;
  }

  function setStatus(status: MetadataStatus | null) {
    activeStatus = activeStatus === status ? null : status;
  }

  function setAuthor(author: RelationFilter | null) {
    activeAuthor = author;
  }

  function setSeries(series: RelationFilter | null) {
    activeSeries = series;
  }

  function setPublisher(publisher: RelationFilter | null) {
    activePublisher = publisher;
  }

  function addTag(tag: TagFilter) {
    if (!activeTags.some((t) => t.id === tag.id)) {
      activeTags = [...activeTags, tag];
    }
  }

  function removeTag(tagId: string) {
    activeTags = activeTags.filter((t) => t.id !== tagId);
  }

  function setTagMatch(mode: TagMatchMode) {
    activeTagMatch = mode;
  }

  function setTrusted(v: boolean | null) {
    activeTrusted = v;
  }

  function setLocked(v: boolean | null) {
    activeLocked = v;
  }

  function setResolutionState(v: ResolutionState | null) {
    activeResolutionState = v;
  }

  function setResolutionOutcome(v: ResolutionOutcome | null) {
    activeResolutionOutcome = v;
  }

  function setLanguage(v: string | null) {
    activeLanguage = v;
  }

  function setYearMin(v: number | null) {
    activeYearMin = v;
  }

  function setYearMax(v: number | null) {
    activeYearMax = v;
  }

  function setHasCover(v: boolean | null) {
    activeHasCover = v;
  }

  function setHasDescription(v: boolean | null) {
    activeHasDescription = v;
  }

  function setHasIdentifiers(v: boolean | null) {
    activeHasIdentifiers = v;
  }

  function setIdentifier(type: IdentifierType | null, value: string) {
    activeIdentifierType = type;
    activeIdentifierValue = value;
  }

  function clearIdentifier() {
    activeIdentifierType = null;
    activeIdentifierValue = '';
  }

  function clearFilters() {
    activeFormat = null;
    activeStatus = null;
    activeAuthor = null;
    activeSeries = null;
    activePublisher = null;
    activeTags = [];
    activeTagMatch = 'any';
    activeTrusted = null;
    activeLocked = null;
    activeResolutionState = null;
    activeResolutionOutcome = null;
    activeLanguage = null;
    activeYearMin = null;
    activeYearMax = null;
    activeHasCover = null;
    activeHasDescription = null;
    activeHasIdentifiers = null;
    activeIdentifierType = null;
    activeIdentifierValue = '';
  }

  /** Build `BookListParams` filter fields (excludes pagination/sort/include). */
  function toListParams(): Partial<BookListParams> {
    const params: Partial<BookListParams> = {};
    if (activeFormat) params.format = activeFormat;
    if (activeStatus) params.status = activeStatus;
    if (activeAuthor) params.author_id = activeAuthor.id;
    if (activeSeries) params.series_id = activeSeries.id;
    if (activePublisher) params.publisher_id = activePublisher.id;
    if (activeTags.length > 0) {
      params.tags = activeTags.map((t) => t.id).join(',');
      if (activeTagMatch !== 'any') params.tag_match = activeTagMatch;
    }
    if (activeTrusted !== null) params.trusted = activeTrusted;
    if (activeLocked !== null) params.locked = activeLocked;
    if (activeResolutionState) params.resolution_state = activeResolutionState;
    if (activeResolutionOutcome) params.resolution_outcome = activeResolutionOutcome;
    if (activeLanguage) params.language = activeLanguage;
    if (activeYearMin !== null) params.year_min = activeYearMin;
    if (activeYearMax !== null) params.year_max = activeYearMax;
    if (activeHasCover !== null) params.has_cover = activeHasCover;
    if (activeHasDescription !== null) params.has_description = activeHasDescription;
    if (activeHasIdentifiers !== null) params.has_identifiers = activeHasIdentifiers;
    if (activeIdentifierType && activeIdentifierValue.trim()) {
      params.identifier_type = activeIdentifierType;
      params.identifier_value = activeIdentifierValue.trim();
    } else if (activeIdentifierValue.trim()) {
      params.identifier_value = activeIdentifierValue.trim();
    }
    return params;
  }

  /** Build a canonical `LibraryFilterState` for scope issuance.
   *  `textQuery` is provided by the caller (the search bar is outside the filter store). */
  function toFilterState(textQuery?: string): LibraryFilterState {
    return {
      text_query: textQuery?.trim() || null,
      author_id: activeAuthor?.id ?? null,
      series_id: activeSeries?.id ?? null,
      publisher_id: activePublisher?.id ?? null,
      tag_ids: activeTags.map((t) => t.id),
      tag_match: activeTagMatch,
      format: activeFormat,
      metadata_status: activeStatus,
      resolution_state: activeResolutionState,
      resolution_outcome: activeResolutionOutcome,
      trusted: activeTrusted,
      locked: activeLocked,
      language: activeLanguage,
      year_min: activeYearMin,
      year_max: activeYearMax,
      has_cover: activeHasCover,
      has_description: activeHasDescription,
      has_identifiers: activeHasIdentifiers,
      identifier_type: activeIdentifierType,
      identifier_value: activeIdentifierValue.trim() || null
    };
  }

  /** Serialize filter state to URL search params. */
  function toURLParams(params: URLSearchParams) {
    if (activeFormat) params.set('format', activeFormat);
    if (activeStatus) params.set('status', activeStatus);
    if (activeAuthor) {
      params.set('author_id', activeAuthor.id);
      params.set('author_name', activeAuthor.name);
    }
    if (activeSeries) {
      params.set('series_id', activeSeries.id);
      params.set('series_name', activeSeries.name);
    }
    if (activePublisher) {
      params.set('publisher_id', activePublisher.id);
      params.set('publisher_name', activePublisher.name);
    }
    if (activeTags.length > 0) {
      params.set('tags', activeTags.map((t) => t.id).join(','));
      params.set('tag_names', activeTags.map((t) => t.name).join(','));
      if (activeTagMatch !== 'any') params.set('tag_match', activeTagMatch);
    }
    if (activeTrusted !== null) params.set('trusted', String(activeTrusted));
    if (activeLocked !== null) params.set('locked', String(activeLocked));
    if (activeResolutionState) params.set('resolution_state', activeResolutionState);
    if (activeResolutionOutcome) params.set('resolution_outcome', activeResolutionOutcome);
    if (activeLanguage) params.set('language', activeLanguage);
    if (activeYearMin !== null) params.set('year_min', String(activeYearMin));
    if (activeYearMax !== null) params.set('year_max', String(activeYearMax));
    if (activeHasCover !== null) params.set('has_cover', String(activeHasCover));
    if (activeHasDescription !== null) params.set('has_description', String(activeHasDescription));
    if (activeHasIdentifiers !== null) params.set('has_identifiers', String(activeHasIdentifiers));
    if (activeIdentifierType && activeIdentifierValue.trim()) {
      params.set('identifier_type', activeIdentifierType);
      params.set('identifier_value', activeIdentifierValue.trim());
    } else if (activeIdentifierValue.trim()) {
      params.set('identifier_value', activeIdentifierValue.trim());
    }
  }

  /** Restore filter state from URL search params. */
  function fromURLParams(params: URLSearchParams) {
    clearFilters();

    const fmt = params.get('format') as BookFormat | null;
    if (fmt) activeFormat = fmt;

    const st = params.get('status') as MetadataStatus | null;
    if (st) activeStatus = st;

    const authorId = params.get('author_id');
    const authorName = params.get('author_name');
    if (authorId && authorName) activeAuthor = { id: authorId, name: authorName };

    const seriesId = params.get('series_id');
    const seriesName = params.get('series_name');
    if (seriesId && seriesName) activeSeries = { id: seriesId, name: seriesName };

    const pubId = params.get('publisher_id');
    const pubName = params.get('publisher_name');
    if (pubId && pubName) activePublisher = { id: pubId, name: pubName };

    const tagIds = params.get('tags');
    const tagNames = params.get('tag_names');
    if (tagIds && tagNames) {
      const ids = tagIds.split(',');
      const names = tagNames.split(',');
      activeTags = ids.map((id, i) => ({ id, name: names[i] || id, category: null }));
    }
    const tm = params.get('tag_match') as TagMatchMode | null;
    if (tm) activeTagMatch = tm;

    const trusted = params.get('trusted');
    if (trusted !== null) activeTrusted = trusted === 'true';

    const locked = params.get('locked');
    if (locked !== null) activeLocked = locked === 'true';

    const rs = params.get('resolution_state') as ResolutionState | null;
    if (rs) activeResolutionState = rs;

    const ro = params.get('resolution_outcome') as ResolutionOutcome | null;
    if (ro) activeResolutionOutcome = ro;

    const lang = params.get('language');
    if (lang) activeLanguage = lang;

    const yMin = params.get('year_min');
    if (yMin) activeYearMin = parseInt(yMin, 10) || null;

    const yMax = params.get('year_max');
    if (yMax) activeYearMax = parseInt(yMax, 10) || null;

    const hc = params.get('has_cover');
    if (hc !== null) activeHasCover = hc === 'true';

    const hd = params.get('has_description');
    if (hd !== null) activeHasDescription = hd === 'true';

    const hi = params.get('has_identifiers');
    if (hi !== null) activeHasIdentifiers = hi === 'true';

    const idType = params.get('identifier_type') as IdentifierType | null;
    const idValue = params.get('identifier_value');
    if (idType && idValue) {
      activeIdentifierType = idType;
      activeIdentifierValue = idValue;
    } else if (idValue) {
      activeIdentifierType = null;
      activeIdentifierValue = idValue;
    }
  }

  /** Return a snapshot key for change detection. */
  function snapshotKey(): string {
    return JSON.stringify([
      activeFormat,
      activeStatus,
      activeAuthor?.id,
      activeSeries?.id,
      activePublisher?.id,
      activeTags.map((t) => t.id),
      activeTagMatch,
      activeTrusted,
      activeLocked,
      activeResolutionState,
      activeResolutionOutcome,
      activeLanguage,
      activeYearMin,
      activeYearMax,
      activeHasCover,
      activeHasDescription,
      activeHasIdentifiers,
      activeIdentifierType,
      activeIdentifierValue
    ]);
  }

  return {
    get activeFormat() {
      return activeFormat;
    },
    get activeStatus() {
      return activeStatus;
    },
    get activeAuthor() {
      return activeAuthor;
    },
    get activeSeries() {
      return activeSeries;
    },
    get activePublisher() {
      return activePublisher;
    },
    get activeTags() {
      return activeTags;
    },
    get activeTagMatch() {
      return activeTagMatch;
    },
    get activeTrusted() {
      return activeTrusted;
    },
    get activeLocked() {
      return activeLocked;
    },
    get activeResolutionState() {
      return activeResolutionState;
    },
    get activeResolutionOutcome() {
      return activeResolutionOutcome;
    },
    get activeLanguage() {
      return activeLanguage;
    },
    get activeYearMin() {
      return activeYearMin;
    },
    get activeYearMax() {
      return activeYearMax;
    },
    get activeHasCover() {
      return activeHasCover;
    },
    get activeHasDescription() {
      return activeHasDescription;
    },
    get activeHasIdentifiers() {
      return activeHasIdentifiers;
    },
    get activeIdentifierType() {
      return activeIdentifierType;
    },
    get activeIdentifierValue() {
      return activeIdentifierValue;
    },
    get hasActiveFilters() {
      return hasActiveFilters;
    },
    get activeFilterCount() {
      return activeFilterCount;
    },
    setFormat,
    setStatus,
    setAuthor,
    setSeries,
    setPublisher,
    addTag,
    removeTag,
    setTagMatch,
    setTrusted,
    setLocked,
    setResolutionState,
    setResolutionOutcome,
    setLanguage,
    setYearMin,
    setYearMax,
    setHasCover,
    setHasDescription,
    setHasIdentifiers,
    setIdentifier,
    clearIdentifier,
    clearFilters,
    toListParams,
    toFilterState,
    toURLParams,
    fromURLParams,
    snapshotKey
  };
}

export const filters = createFilterStore();
