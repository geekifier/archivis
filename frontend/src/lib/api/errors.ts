import type { ApiErrorResponse } from './types.js';

/** A structured error from the API with status code and message. */
export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }

  /** True if this is a 401 Unauthorized response. */
  get isUnauthorized(): boolean {
    return this.status === 401;
  }

  /** True if this is a 403 Forbidden response. */
  get isForbidden(): boolean {
    return this.status === 403;
  }

  /** True if this is a 404 Not Found response. */
  get isNotFound(): boolean {
    return this.status === 404;
  }

  /** True if this is a 409 Conflict response. */
  get isConflict(): boolean {
    return this.status === 409;
  }

  /** A user-facing description suitable for toasts or notifications. */
  get userMessage(): string {
    switch (this.status) {
      case 400:
        return this.message || 'Invalid request. Please check your input.';
      case 401:
        return 'Please log in to continue.';
      case 403:
        return 'You do not have permission to perform this action.';
      case 404:
        return 'The requested resource was not found.';
      case 409:
        return this.message || 'A conflict occurred. The resource may already exist.';
      case 422:
        return this.message || 'The request could not be processed.';
      default:
        if (this.status >= 500) {
          return 'An unexpected server error occurred. Please try again later.';
        }
        return this.message || 'An unexpected error occurred.';
    }
  }
}

/**
 * Parse an API error response body into an `ApiError`.
 * Falls back to a generic error if the body doesn't match the expected format.
 */
export async function parseApiError(response: Response): Promise<ApiError> {
  try {
    const body = (await response.json()) as ApiErrorResponse;
    if (body?.error?.message) {
      return new ApiError(body.error.status ?? response.status, body.error.message);
    }
  } catch {
    // Body wasn't JSON or didn't match expected format
  }
  return new ApiError(response.status, response.statusText || 'Request failed');
}
