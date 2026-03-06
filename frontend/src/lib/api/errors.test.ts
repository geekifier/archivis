import { describe, it, expect } from 'vitest';
import { ApiError, parseApiError } from './errors.js';

describe('ApiError', () => {
  it('sets status and message via constructor', () => {
    const err = new ApiError(400, 'Bad request');
    expect(err.status).toBe(400);
    expect(err.message).toBe('Bad request');
  });

  it('has name property set to ApiError', () => {
    const err = new ApiError(500, 'fail');
    expect(err.name).toBe('ApiError');
  });

  it('extends Error', () => {
    const err = new ApiError(500, 'fail');
    expect(err).toBeInstanceOf(Error);
  });

  describe('isUnauthorized', () => {
    it('returns true for 401', () => {
      expect(new ApiError(401, '').isUnauthorized).toBe(true);
    });

    it('returns false for other status codes', () => {
      expect(new ApiError(403, '').isUnauthorized).toBe(false);
      expect(new ApiError(200, '').isUnauthorized).toBe(false);
    });
  });

  describe('isForbidden', () => {
    it('returns true for 403', () => {
      expect(new ApiError(403, '').isForbidden).toBe(true);
    });

    it('returns false for other status codes', () => {
      expect(new ApiError(401, '').isForbidden).toBe(false);
    });
  });

  describe('isNotFound', () => {
    it('returns true for 404', () => {
      expect(new ApiError(404, '').isNotFound).toBe(true);
    });

    it('returns false for other status codes', () => {
      expect(new ApiError(400, '').isNotFound).toBe(false);
    });
  });

  describe('isConflict', () => {
    it('returns true for 409', () => {
      expect(new ApiError(409, '').isConflict).toBe(true);
    });

    it('returns false for other status codes', () => {
      expect(new ApiError(404, '').isConflict).toBe(false);
    });
  });

  describe('userMessage', () => {
    it('returns message for 400 when message is set', () => {
      expect(new ApiError(400, 'Invalid email').userMessage).toBe('Invalid email');
    });

    it('returns fallback for 400 when message is empty', () => {
      expect(new ApiError(400, '').userMessage).toBe('Invalid request. Please check your input.');
    });

    it('returns fixed message for 401', () => {
      expect(new ApiError(401, 'anything').userMessage).toBe('Please log in to continue.');
    });

    it('returns fixed message for 403', () => {
      expect(new ApiError(403, 'anything').userMessage).toBe(
        'You do not have permission to perform this action.'
      );
    });

    it('returns fixed message for 404', () => {
      expect(new ApiError(404, 'anything').userMessage).toBe(
        'The requested resource was not found.'
      );
    });

    it('returns message for 409 when message is set', () => {
      expect(new ApiError(409, 'Duplicate entry').userMessage).toBe('Duplicate entry');
    });

    it('returns fallback for 409 when message is empty', () => {
      expect(new ApiError(409, '').userMessage).toBe(
        'A conflict occurred. The resource may already exist.'
      );
    });

    it('returns message for 422 when message is set', () => {
      expect(new ApiError(422, 'Validation failed').userMessage).toBe('Validation failed');
    });

    it('returns fallback for 422 when message is empty', () => {
      expect(new ApiError(422, '').userMessage).toBe('The request could not be processed.');
    });

    it('returns server error message for 500', () => {
      expect(new ApiError(500, 'anything').userMessage).toBe(
        'An unexpected server error occurred. Please try again later.'
      );
    });

    it('returns server error message for 502', () => {
      expect(new ApiError(502, '').userMessage).toBe(
        'An unexpected server error occurred. Please try again later.'
      );
    });

    it('returns message for unknown status when message is set', () => {
      expect(new ApiError(418, "I'm a teapot").userMessage).toBe("I'm a teapot");
    });

    it('returns fallback for unknown status when message is empty', () => {
      expect(new ApiError(418, '').userMessage).toBe('An unexpected error occurred.');
    });
  });
});

describe('parseApiError', () => {
  it('parses a response with JSON error body', async () => {
    const response = new Response(
      JSON.stringify({ error: { status: 400, message: 'Bad input' } }),
      { status: 400, headers: { 'Content-Type': 'application/json' } }
    );
    const err = await parseApiError(response);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(400);
    expect(err.message).toBe('Bad input');
  });

  it('falls back to statusText when body is not JSON', async () => {
    const response = new Response('Not JSON', {
      status: 500,
      statusText: 'Internal Server Error'
    });
    const err = await parseApiError(response);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(500);
    expect(err.message).toBe('Internal Server Error');
  });

  it('falls back when body JSON does not have error.message', async () => {
    const response = new Response(JSON.stringify({ something: 'else' }), {
      status: 422,
      statusText: 'Unprocessable Entity',
      headers: { 'Content-Type': 'application/json' }
    });
    const err = await parseApiError(response);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(422);
    expect(err.message).toBe('Unprocessable Entity');
  });

  it('uses response.status if body.error.status is missing', async () => {
    const response = new Response(JSON.stringify({ error: { message: 'Something went wrong' } }), {
      status: 503,
      headers: { 'Content-Type': 'application/json' }
    });
    const err = await parseApiError(response);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(503);
    expect(err.message).toBe('Something went wrong');
  });
});
