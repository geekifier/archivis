import { describe, it, expect, beforeEach } from 'vitest';
import { filters } from './filters.svelte.js';

describe('filters store', () => {
	beforeEach(() => {
		filters.clearFilters();
	});

	it('has correct initial state', () => {
		expect(filters.activeFormat).toBeNull();
		expect(filters.activeStatus).toBeNull();
		expect(filters.hasActiveFilters).toBe(false);
	});

	describe('setFormat', () => {
		it('sets activeFormat and activates filters', () => {
			filters.setFormat('epub');
			expect(filters.activeFormat).toBe('epub');
			expect(filters.hasActiveFilters).toBe(true);
		});

		it('toggles off when setting the same format again', () => {
			filters.setFormat('epub');
			filters.setFormat('epub');
			expect(filters.activeFormat).toBeNull();
			expect(filters.hasActiveFilters).toBe(false);
		});

		it('replaces format when setting a different one', () => {
			filters.setFormat('epub');
			filters.setFormat('pdf');
			expect(filters.activeFormat).toBe('pdf');
		});
	});

	describe('setStatus', () => {
		it('sets activeStatus and activates filters', () => {
			filters.setStatus('identified');
			expect(filters.activeStatus).toBe('identified');
			expect(filters.hasActiveFilters).toBe(true);
		});

		it('toggles off when setting the same status again', () => {
			filters.setStatus('identified');
			filters.setStatus('identified');
			expect(filters.activeStatus).toBeNull();
			expect(filters.hasActiveFilters).toBe(false);
		});
	});

	it('hasActiveFilters is true when both format and status are set', () => {
		filters.setFormat('epub');
		filters.setStatus('identified');
		expect(filters.hasActiveFilters).toBe(true);
	});

	describe('clearFilters', () => {
		it('resets both format and status to null', () => {
			filters.setFormat('epub');
			filters.setStatus('identified');
			filters.clearFilters();
			expect(filters.activeFormat).toBeNull();
			expect(filters.activeStatus).toBeNull();
			expect(filters.hasActiveFilters).toBe(false);
		});
	});

	describe('count setters', () => {
		it('setNeedsReviewCount updates needsReviewCount', () => {
			filters.setNeedsReviewCount(5);
			expect(filters.needsReviewCount).toBe(5);
		});

		it('setUnidentifiedCount updates unidentifiedCount', () => {
			filters.setUnidentifiedCount(10);
			expect(filters.unidentifiedCount).toBe(10);
		});
	});
});
