import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '../../test/utils';
import { ConfirmModal } from './ConfirmModal';

describe('ConfirmModal', () => {
  const defaultProps = {
    isOpen: true,
    title: 'Delete Strategy',
    message: 'Are you sure you want to delete this strategy?',
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders title and message when open', () => {
    render(<ConfirmModal {...defaultProps} />);

    expect(screen.getByText('Delete Strategy')).toBeInTheDocument();
    expect(screen.getByText('Are you sure you want to delete this strategy?')).toBeInTheDocument();
  });

  it('does not render when closed', () => {
    render(<ConfirmModal {...defaultProps} isOpen={false} />);

    expect(screen.queryByText('Delete Strategy')).not.toBeInTheDocument();
  });

  it('renders default button labels', () => {
    render(<ConfirmModal {...defaultProps} />);

    expect(screen.getByText('Confirm')).toBeInTheDocument();
    expect(screen.getByText('Cancel')).toBeInTheDocument();
  });

  it('renders custom button labels', () => {
    render(
      <ConfirmModal
        {...defaultProps}
        confirmLabel="Delete"
        cancelLabel="Keep"
      />
    );

    expect(screen.getByText('Delete')).toBeInTheDocument();
    expect(screen.getByText('Keep')).toBeInTheDocument();
  });

  it('calls onConfirm when confirm button is clicked', () => {
    const onConfirm = vi.fn();
    render(<ConfirmModal {...defaultProps} onConfirm={onConfirm} />);

    fireEvent.click(screen.getByText('Confirm'));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('calls onCancel when cancel button is clicked', () => {
    const onCancel = vi.fn();
    render(<ConfirmModal {...defaultProps} onCancel={onCancel} />);

    fireEvent.click(screen.getByText('Cancel'));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('calls onCancel when backdrop is clicked', () => {
    const onCancel = vi.fn();
    render(<ConfirmModal {...defaultProps} onCancel={onCancel} />);

    // Click the outer overlay (backdrop)
    const backdrop = screen.getByText('Delete Strategy').parentElement?.parentElement;
    if (backdrop) {
      fireEvent.click(backdrop);
      expect(onCancel).toHaveBeenCalledTimes(1);
    }
  });

  it('calls onCancel when Escape key is pressed', () => {
    const onCancel = vi.fn();
    render(<ConfirmModal {...defaultProps} onCancel={onCancel} />);

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  describe('dont ask again checkbox', () => {
    it('shows checkbox when showDontAskAgain is true', () => {
      render(<ConfirmModal {...defaultProps} showDontAskAgain />);

      expect(screen.getByText("Don't show this warning again")).toBeInTheDocument();
      expect(screen.getByRole('checkbox')).toBeInTheDocument();
    });

    it('hides checkbox when showDontAskAgain is false', () => {
      render(<ConfirmModal {...defaultProps} showDontAskAgain={false} />);

      expect(screen.queryByText("Don't show this warning again")).not.toBeInTheDocument();
    });

    it('calls onDontAskAgainChange when checkbox is checked and confirm clicked', () => {
      const onDontAskAgainChange = vi.fn();
      render(
        <ConfirmModal
          {...defaultProps}
          showDontAskAgain
          onDontAskAgainChange={onDontAskAgainChange}
        />
      );

      // Check the checkbox
      fireEvent.click(screen.getByRole('checkbox'));
      // Click confirm
      fireEvent.click(screen.getByText('Confirm'));

      expect(onDontAskAgainChange).toHaveBeenCalledWith(true);
    });

    it('does not call onDontAskAgainChange when checkbox is unchecked', () => {
      const onDontAskAgainChange = vi.fn();
      render(
        <ConfirmModal
          {...defaultProps}
          showDontAskAgain
          onDontAskAgainChange={onDontAskAgainChange}
        />
      );

      // Click confirm without checking the box
      fireEvent.click(screen.getByText('Confirm'));

      expect(onDontAskAgainChange).not.toHaveBeenCalled();
    });
  });

  describe('variants', () => {
    it('applies danger variant styles by default', () => {
      render(<ConfirmModal {...defaultProps} />);

      const confirmButton = screen.getByText('Confirm');
      expect(confirmButton).toHaveClass('bg-red-600');
    });

    it('applies warning variant styles', () => {
      render(<ConfirmModal {...defaultProps} variant="warning" />);

      const confirmButton = screen.getByText('Confirm');
      expect(confirmButton).toHaveClass('bg-yellow-600');
    });

    it('applies info variant styles', () => {
      render(<ConfirmModal {...defaultProps} variant="info" />);

      const confirmButton = screen.getByText('Confirm');
      expect(confirmButton).toHaveClass('bg-blue-600');
    });
  });
});
