import {
  getModelDisplayInfo,
  getRecommendedModel,
  PARAKEET_MODEL_CONFIGS,
} from '../parakeet';

describe('Parakeet model metadata', () => {
  it('keeps the lightweight ONNX Parakeet model as the recommended default', () => {
    expect(getRecommendedModel()).toBe('parakeet-tdt-0.6b-v3-int8');
  });

  it('exposes NVIDIA Parakeet RNNT 1.1B as an opt-in NeMo model', () => {
    const config = PARAKEET_MODEL_CONFIGS['nvidia/parakeet-rnnt-1.1b'];
    const display = getModelDisplayInfo('nvidia/parakeet-rnnt-1.1b');

    expect(config).toMatchObject({
      runtime: 'nemo',
      repo_id: 'nvidia/parakeet-rnnt-1.1b',
      filename: 'parakeet-rnnt-1.1b.nemo',
      size_mb: 4280,
    });
    expect(display?.friendlyName).toBe('RNNT 1.1B');
  });
});
