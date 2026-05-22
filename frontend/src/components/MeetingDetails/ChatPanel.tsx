"use client";

import { useState, useRef, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Send, User, Bot, Loader2, Sparkles } from 'lucide-react';
import { ChatMessage } from '@/types';
import { chatService } from '@/services/chatService';
import { toast } from 'sonner';

interface ChatPanelProps {
  meetingId: string;
  serverAddress: string;
}

export function ChatPanel({ meetingId, serverAddress }: ChatPanelProps) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, isLoading]);

  const handleSendMessage = async (queryText?: string) => {
    const query = queryText || input.trim();
    if (!query || isLoading) return;

    const userMessage: ChatMessage = { role: 'user', content: query };
    setMessages((prev) => [...prev, userMessage]);
    setInput('');
    setIsLoading(true);

    try {
      const response = await chatService.sendChatMessage(serverAddress, {
        meeting_id: meetingId,
        query: query,
        history: messages,
      });

      const assistantMessage: ChatMessage = { role: 'assistant', content: response };
      setMessages((prev) => [...prev, assistantMessage]);
    } catch (error) {
      toast.error('Failed to get response from AI');
      console.error(error);
    } finally {
      setIsLoading(false);
    }
  };

  const quickActions = [
    "What were the key decisions?",
    "List all action items",
    "Who attended the meeting?",
    "Summarize the next steps",
  ];

  return (
    <div className="flex flex-col h-full bg-white">
      {/* Message Area */}
      <div 
        ref={scrollRef}
        className="flex-1 overflow-y-auto p-4 space-y-4 scroll-smooth"
      >
        {messages.length === 0 && !isLoading && (
          <div className="flex flex-col items-center justify-center h-full text-gray-400 space-y-4 opacity-60">
            <Sparkles size={48} />
            <div className="text-center">
              <p className="text-lg font-medium">Chat with your meeting</p>
              <p className="text-sm">Ask anything about the transcript or summary</p>
            </div>
            
            <div className="flex flex-wrap justify-center gap-2 pt-4 max-w-md">
              {quickActions.map((action, i) => (
                <button
                  key={i}
                  onClick={() => handleSendMessage(action)}
                  className="px-3 py-1.5 text-xs bg-gray-100 hover:bg-blue-50 hover:text-blue-600 rounded-full border border-gray-200 transition-colors"
                >
                  {action}
                </button>
              ))}
            </div>
          </div>
        )}

        <AnimatePresence initial={false}>
          {messages.map((message, index) => (
            <motion.div
              key={index}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
            >
              <div className={`flex max-w-[85%] ${message.role === 'user' ? 'flex-row-reverse' : 'flex-row'} gap-2`}>
                <div className={`flex-shrink-0 w-8 h-8 rounded-full flex items-center justify-center ${
                  message.role === 'user' ? 'bg-blue-100 text-blue-600' : 'bg-gray-100 text-gray-600'
                }`}>
                  {message.role === 'user' ? <User size={16} /> : <Bot size={16} />}
                </div>
                <div className={`p-3 rounded-2xl ${
                  message.role === 'user' 
                    ? 'bg-blue-600 text-white rounded-tr-none' 
                    : 'bg-gray-100 text-gray-800 rounded-tl-none'
                }`}>
                  <p className="text-sm whitespace-pre-wrap leading-relaxed">{message.content}</p>
                </div>
              </div>
            </motion.div>
          ))}
        </AnimatePresence>

        {isLoading && (
          <div className="flex justify-start">
            <div className="flex max-w-[85%] gap-2">
              <div className="flex-shrink-0 w-8 h-8 rounded-full bg-gray-100 text-gray-600 flex items-center justify-center">
                <Bot size={16} />
              </div>
              <div className="bg-gray-100 p-4 rounded-2xl rounded-tl-none flex items-center gap-2">
                <Loader2 size={16} className="animate-spin text-gray-400" />
                <span className="text-sm text-gray-500">Thinking...</span>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Input Area */}
      <div className="p-4 border-t border-gray-100 bg-gray-50/50">
        <form 
          onSubmit={(e) => { e.preventDefault(); handleSendMessage(); }}
          className="relative flex items-center"
        >
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Ask a question about this meeting..."
            className="w-full bg-white border border-gray-200 rounded-xl py-3 pl-4 pr-12 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/20 focus:border-blue-500 transition-all shadow-sm"
            disabled={isLoading}
          />
          <button
            type="submit"
            disabled={!input.trim() || isLoading}
            className="absolute right-2 p-2 text-blue-600 hover:bg-blue-50 disabled:text-gray-300 disabled:hover:bg-transparent rounded-lg transition-colors"
          >
            <Send size={18} />
          </button>
        </form>
        <p className="text-[10px] text-gray-400 mt-2 text-center">
          AI generated response. Verify important information.
        </p>
      </div>
    </div>
  );
}
