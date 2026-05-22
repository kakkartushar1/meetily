import { ChatRequest, ChatResponse } from '../types';

export const chatService = {
  /**
   * Send a chat query to the backend for a specific meeting
   */
  sendChatMessage: async (serverAddress: string, request: ChatRequest): Promise<string> => {
    try {
      const response = await fetch(`${serverAddress}/chat`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(request),
      });

      if (!response.ok) {
        const errorData = await response.json();
        throw new Error(errorData.detail || 'Failed to send chat message');
      }

      const data: ChatResponse = await response.json();
      return data.response;
    } catch (error) {
      console.error('Chat service error:', error);
      throw error;
    }
  },
};
