from pydantic import BaseModel
from typing import List, Tuple, Literal
from pydantic_ai import Agent
from pydantic_ai.models.anthropic import AnthropicModel
from pydantic_ai.models.groq import GroqModel
from pydantic_ai.models.openai import OpenAIModel
from pydantic_ai.providers.openai import OpenAIProvider
from pydantic_ai.providers.groq import GroqProvider
from pydantic_ai.providers.anthropic import AnthropicProvider

import logging
import os
from dotenv import load_dotenv
from db import DatabaseManager
from ollama import chat
import asyncio
from ollama import AsyncClient





# Set up logging
logging.basicConfig(
    level=logging.DEBUG,
    format='%(asctime)s - %(levelname)s - [%(filename)s:%(lineno)d] - %(message)s'
)
logger = logging.getLogger(__name__)

load_dotenv()  # Load environment variables from .env file

db = DatabaseManager()

class Block(BaseModel):
    """Represents a block of content in a section.
    
    Block types must align with frontend rendering capabilities:
    - 'text': Plain text content
    - 'bullet': Bulleted list item
    - 'heading1': Large section heading
    - 'heading2': Medium section heading
    
    Colors currently supported:
    - 'gray': Gray text color
    - '' or any other value: Default text color
    """
    id: str
    type: Literal['bullet', 'heading1', 'heading2', 'text']
    content: str
    color: str  # Frontend currently only uses 'gray' or default

class Section(BaseModel):
    """Represents a section in the meeting summary"""
    title: str
    blocks: List[Block]

class MeetingNotes(BaseModel):
    """Represents the meeting notes"""
    meeting_name: str
    sections: List[Section]

class People(BaseModel):
    """Represents the people in the meeting. Always have this part in the output. Title - Person Name (Role, Details)"""
    title: str
    blocks: List[Block]

class SummaryResponse(BaseModel):
    """Represents the meeting summary response based on a section of the transcript"""
    MeetingName : str
    People : People
    SessionSummary : Section
    CriticalDeadlines: Section
    KeyItemsDecisions: Section
    ImmediateActionItems: Section
    NextSteps: Section
    MeetingNotes: MeetingNotes

# --- Main Class Used by main.py ---

class TranscriptProcessor:
    """Handles the processing of meeting transcripts using AI models."""
    def __init__(self):
        """Initialize the transcript processor."""
        logger.info("TranscriptProcessor initialized.")
        self.db = DatabaseManager()
        self.active_clients = []  # Track active Ollama client sessions
        self._use_fallback_key = False  # Stateful per-run: once fallback succeeds, stick to it

    def _create_llm(self, model: str, model_name: str, api_key: str):
        """Create an LLM instance for the given provider and API key."""
        if model == "claude":
            return AnthropicModel(model_name, provider=AnthropicProvider(api_key=api_key))
        elif model == "groq":
            return GroqModel(model_name, provider=GroqProvider(api_key=api_key))
        elif model == "openai":
            return OpenAIModel(model_name, provider=OpenAIProvider(api_key=api_key))
        elif model == "openrouter":
            return OpenAIModel(model_name, provider=OpenAIProvider(
                base_url="https://openrouter.ai/api/v1", api_key=api_key
            ))
        elif model == "gemini":
            return OpenAIModel(model_name, provider=OpenAIProvider(
                base_url="https://generativelanguage.googleapis.com/v1beta/openai/", api_key=api_key
            ))
        else:
            raise ValueError(f"Cannot create LLM for provider: {model}")

    async def _get_fallback_key(self, model: str) -> str:
        """Get the fallback API key for a provider from the database."""
        try:
            fallback_key = await self.db.get_fallback_api_key(model)
            return fallback_key if fallback_key else ""
        except Exception as e:
            logger.warning(f"Failed to get fallback API key for {model}: {e}")
            return ""
    async def process_transcript(self, text: str, model: str, model_name: str, chunk_size: int = 5000, overlap: int = 1000, custom_prompt: str = "") -> Tuple[int, List[str]]:
        """
        Process transcript text into chunks and generate structured summaries for each chunk using an AI model.

        Args:
            text: The transcript text.
            model: The AI model provider ('claude', 'ollama', 'groq', 'openai').
            model_name: The specific model name.
            chunk_size: The size of each text chunk.
            overlap: The overlap between consecutive chunks.
            custom_prompt: A custom prompt to use for the AI model.

        Returns:
            A tuple containing:
            - The number of chunks processed.
            - A list of JSON strings, where each string is the summary of a chunk.
        """

        logger.info(f"Processing transcript (length {len(text)}) with model provider={model}, model_name={model_name}, chunk_size={chunk_size}, overlap={overlap}")

        all_json_data = []
        agent = None # Define agent variable
        llm = None # Define llm variable

        try:
            # Select and initialize the AI model and agent
            primary_api_key = None
            if model == "claude":
                api_key = await db.get_api_key("claude")
                if not api_key: raise ValueError("ANTHROPIC_API_KEY environment variable not set")
                primary_api_key = api_key
                # If we're already using fallback from a previous chunk failure, start with fallback
                if self._use_fallback_key:
                    fallback_key = await self._get_fallback_key(model)
                    if fallback_key:
                        logger.info(f"Using fallback API key for {model} (sticky from previous chunk failure)")
                        api_key = fallback_key
                llm = AnthropicModel(model_name, provider=AnthropicProvider(api_key=api_key))
                logger.info(f"Using Claude model: {model_name}")
            elif model == "ollama":
                # Use environment variable for Ollama host configuration
                ollama_host = os.getenv('OLLAMA_HOST', 'http://localhost:11434')
                ollama_base_url = f"{ollama_host}/v1"
                ollama_model = OpenAIModel(
                    model_name=model_name, provider=OpenAIProvider(base_url=ollama_base_url)
                )
                llm = ollama_model
                if model_name.lower().startswith("phi4") or model_name.lower().startswith("llama"):
                    chunk_size = 10000
                    overlap = 1000
                else:
                    chunk_size = 30000
                    overlap = 1000
                logger.info(f"Using Ollama model: {model_name}")
            elif model == "groq":
                api_key = await db.get_api_key("groq")
                if not api_key: raise ValueError("GROQ_API_KEY environment variable not set")
                primary_api_key = api_key
                if self._use_fallback_key:
                    fallback_key = await self._get_fallback_key(model)
                    if fallback_key:
                        logger.info(f"Using fallback API key for {model} (sticky from previous chunk failure)")
                        api_key = fallback_key
                llm = GroqModel(model_name, provider=GroqProvider(api_key=api_key))
                logger.info(f"Using Groq model: {model_name}")
            elif model == "openai":
                api_key = await db.get_api_key("openai")
                if not api_key: raise ValueError("OPENAI_API_KEY environment variable not set")
                primary_api_key = api_key
                if self._use_fallback_key:
                    fallback_key = await self._get_fallback_key(model)
                    if fallback_key:
                        logger.info(f"Using fallback API key for {model} (sticky from previous chunk failure)")
                        api_key = fallback_key
                llm = OpenAIModel(model_name, provider=OpenAIProvider(api_key=api_key))
                logger.info(f"Using OpenAI model: {model_name}")
            else:
                logger.error(f"Unsupported model provider requested: {model}")
                raise ValueError(f"Unsupported model provider: {model}")

            # Initialize the agent with the selected LLM
            agent = Agent(
                llm,
                result_type=SummaryResponse,
                result_retries=2,
            )
            logger.info("Pydantic-AI Agent initialized.")

            # Split transcript into chunks
            step = chunk_size - overlap
            if step <= 0:
                logger.warning(f"Overlap ({overlap}) >= chunk_size ({chunk_size}). Adjusting overlap.")
                overlap = max(0, chunk_size - 100)
                step = chunk_size - overlap

            chunks = [text[i:i+chunk_size] for i in range(0, len(text), step)]
            num_chunks = len(chunks)
            logger.info(f"Split transcript into {num_chunks} chunks.")

            for i, chunk in enumerate(chunks):
                logger.info(f"Processing chunk {i+1}/{num_chunks}...")
                try:
                    # Run the agent to get the structured summary for the chunk
                    if model != "ollama":
                        summary_result = await agent.run(
                            f"""Given the following meeting transcript chunk, extract the relevant information according to the required JSON structure. If a specific section (like Critical Deadlines) has no relevant information in this chunk, return an empty list for its 'blocks'. Ensure the output is only the JSON data.

                            IMPORTANT: Block types must be one of: 'text', 'bullet', 'heading1', 'heading2'
                            - Use 'text' for regular paragraphs
                            - Use 'bullet' for list items
                            - Use 'heading1' for major headings
                            - Use 'heading2' for subheadings
                            
                            For the color field, use 'gray' for less important content or '' (empty string) for default.

                            Transcript Chunk:
                            ---
                        {chunk}
                        ---

                        Please capture all relevant action items. Transcription can have spelling mistakes. correct it if required. context is important.
                        
                        While generating the summary, please add the following context:
                        ---
                        {custom_prompt}
                        ---
                        Make sure the output is only the JSON data.
                        """,
                    )
                    else:
                        logger.info(f"Using Ollama model: {model_name} and chunk size: {chunk_size} with overlap: {overlap}")
                        response = await self.chat_ollama_model(model_name, chunk, custom_prompt)
                        
                        # Check if response is already a SummaryResponse object or a string that needs validation
                        if isinstance(response, SummaryResponse):
                            summary_result = response
                        else:
                            # If it's a string (JSON), validate it
                            summary_result = SummaryResponse.model_validate_json(response)
                            
                        logger.info(f"Summary result for chunk {i+1}: {summary_result}")
                        logger.info(f"Summary result type for chunk {i+1}: {type(summary_result)}")

                    if hasattr(summary_result, 'data') and isinstance(summary_result.data, SummaryResponse):
                         final_summary_pydantic = summary_result.data
                    elif isinstance(summary_result, SummaryResponse):
                         final_summary_pydantic = summary_result
                    else:
                         logger.error(f"Unexpected result type from agent for chunk {i+1}: {type(summary_result)}")
                         continue # Skip this chunk

                    # Convert the Pydantic model to a JSON string
                    chunk_summary_json = final_summary_pydantic.model_dump_json()
                    all_json_data.append(chunk_summary_json)
                    logger.info(f"Successfully generated summary for chunk {i+1}.")

                except Exception as chunk_error:
                    logger.error(f"Error processing chunk {i+1}: {chunk_error}", exc_info=True)

                    # --- FALLBACK KEY RETRY LOGIC ---
                    # Only attempt fallback for cloud providers (not ollama)
                    if model != "ollama" and not self._use_fallback_key:
                        fallback_key = await self._get_fallback_key(model)
                        if fallback_key:
                            logger.info(f"Primary key failed for chunk {i+1}. Attempting fallback key for {model}...")
                            try:
                                fallback_llm = self._create_llm(model, model_name, fallback_key)
                                fallback_agent = Agent(
                                    fallback_llm,
                                    result_type=SummaryResponse,
                                    result_retries=2,
                                )
                                fallback_result = await fallback_agent.run(
                                    f"""Given the following meeting transcript chunk, extract the relevant information according to the required JSON structure. If a specific section (like Critical Deadlines) has no relevant information in this chunk, return an empty list for its 'blocks'. Ensure the output is only the JSON data.

                                    IMPORTANT: Block types must be one of: 'text', 'bullet', 'heading1', 'heading2'
                                    - Use 'text' for regular paragraphs
                                    - Use 'bullet' for list items
                                    - Use 'heading1' for major headings
                                    - Use 'heading2' for subheadings
                                    
                                    For the color field, use 'gray' for less important content or '' (empty string) for default.

                                    Transcript Chunk:
                                    ---
                                {chunk}
                                ---

                                Please capture all relevant action items. Transcription can have spelling mistakes. correct it if required. context is important.
                                
                                While generating the summary, please add the following context:
                                ---
                                {custom_prompt}
                                ---
                                Make sure the output is only the JSON data.
                                """,
                                )

                                if hasattr(fallback_result, 'data') and isinstance(fallback_result.data, SummaryResponse):
                                    final_summary_pydantic = fallback_result.data
                                elif isinstance(fallback_result, SummaryResponse):
                                    final_summary_pydantic = fallback_result
                                else:
                                    logger.error(f"Unexpected fallback result type for chunk {i+1}: {type(fallback_result)}")
                                    continue

                                chunk_summary_json = final_summary_pydantic.model_dump_json()
                                all_json_data.append(chunk_summary_json)
                                logger.info(f"✅ Fallback key succeeded for chunk {i+1}. Switching to fallback for remaining chunks.")

                                # STATEFUL: Stick to fallback key for all remaining chunks
                                self._use_fallback_key = True
                                # Re-initialize the main agent with fallback key for subsequent chunks
                                llm = fallback_llm
                                agent = fallback_agent

                            except Exception as fallback_error:
                                logger.error(f"Fallback key also failed for chunk {i+1}: {fallback_error}", exc_info=True)
                        else:
                            logger.warning(f"No fallback API key configured for {model}. Skipping chunk {i+1}.")
                    # --- END FALLBACK KEY RETRY LOGIC ---

            logger.info(f"Finished processing all {num_chunks} chunks.")
            # Reset fallback state for next run
            self._use_fallback_key = False
            return num_chunks, all_json_data

        except Exception as e:
            logger.error(f"Error during transcript processing: {str(e)}", exc_info=True)
            raise
    
    async def chat_ollama_model(self, model_name: str, transcript: str, custom_prompt: str):
        message = {
        'role': 'system',
        'content': f'''
        Given the following meeting transcript chunk, extract the relevant information according to the required JSON structure. If a specific section (like Critical Deadlines) has no relevant information in this chunk, return an empty list for its 'blocks'. Ensure the output is only the JSON data.

        Transcript Chunk:
            ---
            {transcript}
            ---
        Please capture all relevant action items. Transcription can have spelling mistakes. correct it if required. context is important.
        
        While generating the summary, please add the following context:
        ---
        {custom_prompt}
        ---

        Make sure the output is only the JSON data.
    
        ''',
        }

        # Create a client and track it for cleanup
        ollama_host = os.getenv('OLLAMA_HOST', 'http://127.0.0.1:11434')
        client = AsyncClient(host=ollama_host)
        self.active_clients.append(client)
        
        try:
            response = await client.chat(model=model_name, messages=[message], stream=True, format=SummaryResponse.model_json_schema())
            
            full_response = ""
            async for part in response:
                content = part['message']['content']
                print(content, end='', flush=True)
                full_response += content
            
            try:
                summary = SummaryResponse.model_validate_json(full_response)
                print("\n", summary.model_dump_json(indent=2), type(summary))
                return summary
            except Exception as e:
                print(f"\nError parsing response: {e}")
                return full_response
        except asyncio.CancelledError:
            logger.info("Ollama request was cancelled during shutdown")
            raise
        except Exception as e:
            logger.error(f"Error in Ollama chat: {e}")
            raise
        finally:
            # Remove the client from active clients list
            if client in self.active_clients:
                self.active_clients.remove(client)

    async def chat(self, query: str, context: str, model: str, model_name: str, history: List[dict] = None) -> str:
        """
        Chat with the meeting content.
        
        Args:
            query: The user's question.
            context: The meeting context (transcript + summary).
            model: The AI model provider.
            model_name: The specific model name.
            history: Previous chat history.
            
        Returns:
            The AI's response text.
        """
        logger.info(f"Chatting with meeting context using model={model}, model_name={model_name}")
        
        try:
            llm = None
            if model == "claude":
                api_key = await db.get_api_key("claude")
                if not api_key: raise ValueError("ANTHROPIC_API_KEY not set")
                llm = AnthropicModel(model_name, provider=AnthropicProvider(api_key=api_key))
            elif model == "ollama":
                ollama_host = os.getenv('OLLAMA_HOST', 'http://localhost:11434')
                llm = OpenAIModel(model_name=model_name, provider=OpenAIProvider(base_url=f"{ollama_host}/v1"))
            elif model == "groq":
                api_key = await db.get_api_key("groq")
                if not api_key: raise ValueError("GROQ_API_KEY not set")
                llm = GroqModel(model_name, provider=GroqProvider(api_key=api_key))
            elif model == "openai":
                api_key = await db.get_api_key("openai")
                if not api_key: raise ValueError("OPENAI_API_KEY not set")
                llm = OpenAIModel(model_name, provider=OpenAIProvider(api_key=api_key))
            else:
                raise ValueError(f"Unsupported model provider: {model}")

            system_prompt = f"""You are an expert meeting assistant. Your goal is to help users get insights from their meeting transcripts and summaries.
            
            Below is the context for the meeting:
            ---
            {context}
            ---
            
            Answer the user's questions precisely based on the provided context. If the information is not in the context, state that you don't know rather than hallucinating.
            """
            
            full_query = query
            if history:
                history_text = "\n".join([f"{m['role'].capitalize()}: {m['content']}" for m in history])
                full_query = f"Previous conversation:\n{history_text}\n\nUser Question: {query}"

            agent = Agent(llm, system_prompt=system_prompt)
            
            try:
                result = await agent.run(full_query)
                return result.data
            except Exception as primary_error:
                logger.error(f"Primary key failed for chat: {primary_error}", exc_info=True)
                
                # --- FALLBACK KEY RETRY LOGIC FOR CHAT ---
                if model != "ollama":
                    fallback_key = await self._get_fallback_key(model)
                    if fallback_key:
                        logger.info(f"Attempting chat with fallback API key for {model}...")
                        try:
                            fallback_llm = self._create_llm(model, model_name, fallback_key)
                            fallback_agent = Agent(fallback_llm, system_prompt=system_prompt)
                            result = await fallback_agent.run(full_query)
                            logger.info(f"✅ Chat succeeded with fallback API key for {model}")
                            return result.data
                        except Exception as fallback_error:
                            logger.error(f"Fallback key also failed for chat: {fallback_error}", exc_info=True)
                            raise fallback_error
                    else:
                        logger.warning(f"No fallback API key configured for {model}")
                
                raise primary_error

        except Exception as e:
            logger.error(f"Error in chat: {str(e)}", exc_info=True)
            raise

    def cleanup(self):
        """Clean up resources used by the TranscriptProcessor."""
        logger.info("Cleaning up TranscriptProcessor resources")
        try:
            # Close database connections if any
            if hasattr(self, 'db') and self.db is not None:
                # self.db.close()
                logger.info("Database connection cleanup (using context managers)")
                
            # Cancel any active Ollama client sessions
            if hasattr(self, 'active_clients') and self.active_clients:
                logger.info(f"Terminating {len(self.active_clients)} active Ollama client sessions")
                for client in self.active_clients:
                    try:
                        # Close the client's underlying connection
                        if hasattr(client, '_client') and hasattr(client._client, 'close'):
                            asyncio.create_task(client._client.aclose())
                    except Exception as client_error:
                        logger.error(f"Error closing Ollama client: {client_error}", exc_info=True)
                # Clear the list
                self.active_clients.clear()
                logger.info("All Ollama client sessions terminated")
        except Exception as e:
            logger.error(f"Error during TranscriptProcessor cleanup: {str(e)}", exc_info=True)

        